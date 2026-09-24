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

use super::emit::{X86Codegen, is_xmm_reg, phys_reg_name, phys_reg_name_256};
use crate::backend::regalloc::PhysReg;
use crate::backend::state::StackSlot;
use crate::common::types::IrType;
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

/// Element width selector for the packed integer compare emitter.
///
/// x86 gives `pcmpeq`/`pcmpgt` a suffix per element width and provides NO
/// unsigned form below AVX-512, so both widths share one implementation that
/// differs only in the mnemonic suffix and in the per-lane sign-bias
/// constant used to remap the unsigned order onto the signed one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum IntCmpLane {
    /// 32-bit lanes: `pcmpeqd` / `pcmpgtd`, bias 0x8000_0000.
    Dword,
    /// 8-bit lanes: `pcmpeqb` / `pcmpgtb`, bias 0x80 in every byte.
    Byte,
    /// 16-bit lanes: `pcmpeqw` / `pcmpgtw`, bias 0x8000 in every word.
    Word,
}

/// Mnemonic domain for the SSE2-baseline lane-mask select (see
/// `emit_sse_blendv_128`): the FP payloads use the legacy PS/PD encodings
/// (domain-matched), integer payloads use pand/pandn/por (integer domain,
/// no float-bypass delay on the lane data).
pub(super) enum BlendvDomain {
    Ps,
    Pd,
    Int,
}

impl X86Codegen {
    /// Emitter-side commutativity table for the 128-bit VEX memfold: may the
    /// folded load sit in the IR args[0] (src1) position? The IR analysis
    /// (`memfold_consumer_128`) is the admission authority and only marks
    /// args[0] for commutative ops; this table is the independent, fail-closed
    /// defense at the point of emission — a mnemonic not listed here is
    /// treated as non-commutative and the fold is materialised instead. The
    /// vocabulary is exactly the set of legacy mnemonics the `Vec*` dispatch
    /// passes to `emit_sse_binary_128` (an unknown mnemonic can only come
    /// from a NEW dispatch site, which must be added here — or reviewed —
    /// deliberately).
    fn sse128_mnemonic_commutative(inst: &str) -> bool {
        matches!(
            inst,
            // Integer/FP add and mul (wrapping/IEEE lane arithmetic).
            "paddb" | "paddw" | "paddd" | "paddq"
                | "pmullw" | "pmulld"
                | "addps" | "addpd" | "mulps" | "mulpd"
                // Bitwise (integer and FP domains alike — pure lane XOR).
                | "pand" | "por" | "pxor" | "xorps" | "xorpd"
                // Integer min/max (no FP unordered/±0 asymmetry).
                | "pminub" | "pmaxub" | "pminsw" | "pmaxsw"
                | "pminsd" | "pmaxsd"
        )
    }
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
                        // The home->scratch copy overwrites `xmm`: every
                        // OTHER value's claim on that bank just died.
                        self.state.vec_evict_bank_except(xmm, v.0);
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
                    // The move-aside overwrites `xmm`: every OTHER value's
                    // claim on that bank just died.
                    self.state.vec_evict_bank_except(xmm, v.0);
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
                    self.state.vec_evict_bank_except(xmm, v.0);
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
                // The load overwrites `xmm` with `v`'s data: every OTHER
                // value's claim on that bank is now stale.
                self.state.vec_evict_bank_except(xmm, v.0);
                self.state.sse_last_store_reg = false;
                return;
            }
        }
        self.flush_pending_vec_store_impl();
        self.operand_to_reg(arg, "rax");
        self.state
            .emit_fmt(format_args!("    movdqu (%rax), %{}", xmm));
        if let Operand::Value(v) = arg {
            self.state.vec_evict_bank_except(xmm, v.0);
        }
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
                self.state.vec_claim_live_reg(dest_ptr.0, name);
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
                // `xmm` now holds THIS value (stored or pending): every
                // OTHER value's claim on that bank is stale — the producing
                // op (pclmulqdq/blend/etc. destructive forms that stage
                // through the scratch) overwrote it (the crc_fold_state1
                // stale-claim class).
                self.state.vec_evict_bank_except(xmm, dest_ptr.0);
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
        self.state.vec_evict_bank_except(xmm, dest_ptr.0);
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
    /// 256-bit or 128-bit load and remember its source memory operand for the
    /// adjacent consumer. Returns `true` when nothing must be emitted for this
    /// intrinsic. Falls back to the ordinary load path unless base/index are
    /// RA-homed GPRs (or a zero constant) — scratch `%rax`/`%rcx` addressing
    /// would not survive the intervening load — and unless the destination
    /// EVEN WHEN the destination has an XMM home.
    ///
    /// Folding removes the load outright, so the comparison is
    ///
    ///   homed load + consumer  ->  `vmovdqu mem, %ymmL` ; `op %ymmL, %ymmS, %ymmD`
    ///   folded load            ->  `op mem, %ymmS, %ymmD`
    ///
    /// one instruction and one live register fewer per iteration.  GCC's byte
    /// clamp is exactly the folded form (`vpminub (%rsi,%rax), %ymm3, %ymm0`).
    ///
    /// 128-bit loads join under `avx2_enabled`: the VEX.128 encoding of the
    /// consumer reads r/m128 with NO alignment requirement, exactly like the
    /// VEX.256 forms (the whole point of VEX). Legacy SSE memory operands
    /// would require 16-byte alignment the streamed objects do not carry, so
    /// the 128-bit families stay ordinary loads without AVX2.
    ///
    /// This used to bail on a homed destination, and that bail WAS
    /// load-bearing -- but the hazard was never the single-use census, it was
    /// operand RESOLUTION.  An elided value has no materialised contents, yet
    /// `vec_home_256`/`vec_home_128`/`vec_arg_mem` reported the allocator's
    /// register (never written) or the home slot (never written) as if it did.
    /// A consumer that did not take the memory-fold path then read garbage:
    /// measured as a wrong FNV hash in `vec_load_sink_memfold` and an outright
    /// failure in `vectorize_map_expr_tree`.
    ///
    /// Those three resolvers now consult `pending_vec_memfold` FIRST -- the
    /// home resolvers report "not homed" and `vec_arg_mem` reports the
    /// recorded source operand -- joining `avx_load_arg_to`, which already
    /// re-issued the load into whatever scratch register the caller asked
    /// for.  With resolution memfold-aware, every consumer path is covered:
    /// it either consumes the memory operand or re-issues the load.  The
    /// register the allocator reserved simply goes unwritten, which is sound
    /// because the value's only use is the consumer doing the folding.
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
        // A register-homed load may only be elided when its consumer is one
        // of the audited memfold-first emitters (see
        // `compute_vector_memfold_homed_ok`).  For every other consumer the
        // home would be reported as live by that consumer's own operand
        // lookup while never having been written.
        if self
            .reg_assignments
            .get(&d.0)
            .is_some_and(|r| is_xmm_reg(*r))
            && !self.state.vector_memfold_homed_ok.contains(&d.0)
        {
            return false;
        }
        let (mnemonic, width): (&'static str, u32) = match op {
            IntrinsicOp::VecLoadF64x4 => ("vmovupd", 32),
            IntrinsicOp::VecLoadF32x8 => ("vmovups", 32),
            // VecLoadI16x16 is the halfword twin of VecLoadI8x32: the same
            // vmovdqu %ymm stream load, the same vec_mem_operand arg layout
            // (base, index, const disp). Without this arm the elision
            // rejected every halfword map loop's stream load — one extra
            // vmovdqu + register-home round trip per fold (Review F1).
            IntrinsicOp::VecLoadI32x8 | IntrinsicOp::VecLoadI8x32 | IntrinsicOp::VecLoadI16x16 => {
                ("vmovdqu", 32)
            }
            IntrinsicOp::VecLoadI64x4 => ("vmovdqu", 32),
            // The 128-bit twins (VEX.128 forms are alignment-free, hence the
            // avx2 gate — see the doc comment above).
            IntrinsicOp::VecLoadF64x2 if self.avx2_enabled => ("vmovupd", 16),
            IntrinsicOp::VecLoadF32x4 if self.avx2_enabled => ("vmovups", 16),
            IntrinsicOp::VecLoadI32x4
            | IntrinsicOp::VecLoadI64x2
            | IntrinsicOp::VecLoadI16x8
            | IntrinsicOp::VecLoadI8x16
                if self.avx2_enabled =>
            {
                ("vmovdqu", 16)
            }
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
            Some(ix) => format!(
                "{}(%{},%{})",
                disp_str,
                phys_reg_name(base),
                phys_reg_name(ix)
            ),
            None => format!("{}(%{})", disp_str, phys_reg_name(base)),
        };
        if std::env::var("CCC_DEBUG_VLFOLD").is_ok() {
            eprintln!("[VLFOLD-EMIT] elide load %{} <- {} {}", d.0, mnemonic, mem);
        }
        self.state.vector_values.insert(d.0);
        self.state.pending_vec_memfold = Some(crate::backend::state::PendingVecMemfold {
            val: d.0,
            mem,
            mnemonic,
            width,
        });
        true
    }

    /// Memory operand of a pending VLFOLD load if `arg` is that value.
    fn memfold_operand(&self, arg: &Operand) -> Option<String> {
        match (arg, &self.state.pending_vec_memfold) {
            (Operand::Value(v), Some(pf)) if v.0 == pf.val => Some(pf.mem.clone()),
            _ => None,
        }
    }

    /// Materialise a pending VLFOLD load through the scratch register and
    /// its ordinary home (register or slot). Never expected on the analysed
    /// shapes; keeps the elision sound if an unexpected instruction
    /// intervenes. WIDTH-EXACT: a 128-bit fold materialises through `%xmm0`
    /// and the SSE store discipline reading ONLY 16 bytes — a 32-byte read
    /// of a 16-byte object could cross a page the program never touched —
    /// and a VEX.128 load zeroes the upper YMM half, so `dirty_upper_ymm`
    /// stays clear exactly like every other 128-bit VEX load.
    pub(super) fn materialize_pending_memfold(&mut self) {
        let Some(pf) = self.state.pending_vec_memfold.take() else {
            return;
        };
        if std::env::var("CCC_DEBUG_VLFOLD").is_ok() {
            eprintln!(
                "[VLFOLD-EMIT] materialising %{} (unexpected consumer)",
                pf.val
            );
        }
        self.flush_pending_vec_store_impl();
        if pf.width == 16 {
            self.state
                .emit_fmt(format_args!("    {} {}, %xmm0", pf.mnemonic, pf.mem));
            self.sse_store_dest(&Value(pf.val), "xmm0");
        } else {
            self.state
                .emit_fmt(format_args!("    {} {}, %ymm0", pf.mnemonic, pf.mem));
            self.state.dirty_upper_ymm = true;
            self.avx_store_dest(&Value(pf.val));
        }
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

        // VLFOLD (memfold-FIRST, before every in-place/accumulator path —
        // the emit_avx_binary_256_inner discipline): an elided single-use
        // 128-bit load becomes the consumer's r/m operand. AT&T VEX order
        // is `vop src2, src1, dst` and only the FIRST textual operand may
        // be memory, so:
        //   * the fold in args[1] (src2) emits `vop MEM, src1, dst` —
        //     legal for EVERY op (the r/m slot is exactly src2);
        //   * the fold in args[0] (src1) is only admitted by the IR
        //     analysis for COMMUTATIVE ops and emits `vop MEM, src2, dst`
        //     (= src2 op MEM = src1 op src2); the mnemonic table below is
        //     the emitter-side defense — an unknown mnemonic materialises
        //     instead of guessing.
        // Width-matched to 16: a foreign-width fold can never reach here
        // (the safety net materialises it first); this is defense.
        if self.avx2_enabled {
            if let Some(pf) = self.state.pending_vec_memfold.clone() {
                if pf.width == 16 {
                    let at1 = matches!(&args[1], Operand::Value(v) if v.0 == pf.val);
                    let at0 = matches!(&args[0], Operand::Value(v) if v.0 == pf.val);
                    if at0 && at1 {
                        // The SAME elided value in both operand positions:
                        // the IR analysis rejects this shape (a0 == a1), so
                        // reaching it means an unanalysed producer —
                        // materialise rather than fold one side and read the
                        // never-written register for the other.
                        self.materialize_pending_memfold();
                    } else if at1 || (at0 && Self::sse128_mnemonic_commutative(sse_inst)) {
                        let dst_home = self.dest_xmm_home_name(dest_ptr);
                        let (mem_operand, reg_operand) = if at1 {
                            let src1 = self.vex128_source(&args[0], "xmm0");
                            (pf.mem, src1)
                        } else {
                            let src2 = self.vex128_source(&args[1], "xmm1");
                            (pf.mem, src2)
                        };
                        let dst = match dst_home {
                            Some(name) => format!("%{}", name),
                            None => "%xmm0".to_string(),
                        };
                        self.state.emit_fmt(format_args!(
                            "    v{} {}, {}, {}",
                            sse_inst, mem_operand, reg_operand, dst
                        ));
                        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                        // Commit BEFORE creating the dest's own deferred
                        // store: the commit clears a pending that names the
                        // committed value (a SOURCE whose data flowed through
                        // the write), and a pending created here belongs to
                        // THIS dest — creating it first made the commit kill
                        // the dest's only written-home (the hsum stale-slot
                        // miscompile).
                        self.sse_commit_dest_direct(dest_ptr, dst_static);
                        if dst_home.is_none() {
                            let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
                            use crate::backend::state::SlotAddr;
                            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                self.state.resolve_slot_addr(dest_ptr.0)
                            {
                                if !deferred {
                                    self.state.emit_fmt(format_args!(
                                        "    movdqu %xmm0, {}",
                                        self.slot_ref(slot.0)
                                    ));
                                } else {
                                    self.state.pending_vec_store =
                                        Some((dest_ptr.0, "xmm0", false));
                                }
                            }
                        }
                        self.state.pending_vec_memfold = None;
                        return;
                    } else if at0 || at1 {
                        // The op names the folded value but cannot fold it
                        // here (non-commutative src1 position, or an
                        // unrecognised mnemonic): materialise the load and
                        // fall through to the ordinary paths.
                        self.materialize_pending_memfold();
                    }
                }
            }
        }

        // In-place two-operand form (destructive-form coalescing in the
        // RA): the destination shares the first operand's home — the first
        // operand's last mention is this very instruction — and the second
        // operand resolves to a register (held deferred value, block-local
        // live register, or home).  One `op %src, %dst`, no staging.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(dest_reg) {
                if let (Operand::Value(a0), Operand::Value(a1)) = (&args[0], &args[1]) {
                    if a0.0 != a1.0 && self.reg_assignments.get(&a0.0) == Some(&dest_reg) {
                        let target = phys_reg_name(dest_reg);
                        if let Some(src) = self.vec_operand_reg(a1) {
                            if src != target {
                                // The consumed second operand's pending
                                // deferred store (single-use by
                                // construction) flowed through its register.
                                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(a1.0) {
                                    self.state.pending_vec_store = None;
                                }
                                // VEX form under AVX2: the legacy 2-operand
                                // `op %src, %target` is SSE-encoded, and
                                // mixing it with the VEX shift/load forms
                                // in one loop pays a SSE→VEX transition
                                // penalty on every execution (the known
                                // mixing follow-up). The non-destructive
                                // encoding with dst == src1 is
                                // semantically identical.
                                if self.avx2_enabled {
                                    self.state.emit_fmt(format_args!(
                                        "    v{} %{}, %{}, %{}",
                                        sse_inst, src, target, target
                                    ));
                                } else {
                                    self.state.emit_fmt(format_args!(
                                        "    {} %{}, %{}",
                                        sse_inst, src, target
                                    ));
                                }
                                self.sse_mark_in_place(dest_ptr, target);
                                return;
                            }
                        }
                    }
                }
            }
        }

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
                        // VEX form under AVX2 (same reasoning as the
                        // in-place path above: no legacy/VEX mixing in
                        // one loop body).
                        if self.avx2_enabled {
                            self.state.emit_fmt(format_args!(
                                "    v{} %{}, %{}, %{}",
                                sse_inst, held, target, target
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    {} %{}, %{}", sse_inst, held, target));
                        }
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(fresh.0) {
                            self.state.pending_vec_store = None;
                        }
                        self.state.vec_claim_live_reg(dest_ptr.0, target);
                        self.state.sse_last_store_val = Some(dest_ptr.0);
                        self.state.sse_last_store_reg = true;
                        self.state.sse_last_store_reg_name = Some(target);
                        return;
                    }
                }
            }
        }

        // VEX.128 three-operand fast path (AVX2 enabled): every SSE2
        // integer/FP SIMD op has a non-destructive VEX.128 encoding
        // `v<op> SRC2, SRC1, DST`.  With both sources resolved to their
        // homes (or the %xmm0/%xmm1 scratch pair) and the destination to
        // its RA home, the op is ONE instruction with zero staging --
        // the register-register chains the destructive-form pre-
        // allocation builds (ARX rounds, accumulator chains) lower
        // without any movdqa at all.  Operand order follows the legacy
        // contract: args[0] is src1, args[1] is src2 (sub: src1-src2).
        if self.avx2_enabled {
            let src1 = self.vex128_source(&args[0], "xmm0");
            let src2 = self.vex128_source(&args[1], "xmm1");
            let dst_home = self.dest_xmm_home_name(dest_ptr);
            let dst = match dst_home {
                Some(name) => format!("%{}", name),
                None => "%xmm0".to_string(),
            };
            if !(src1 == "%xmm0" && src2 == "%xmm0") {
                self.state.emit_fmt(format_args!(
                    "    v{} {}, {}, {}",
                    sse_inst, src2, src1, dst
                ));
                let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                // Commit BEFORE creating the dest's own deferred store: the
                // commit clears a pending naming the committed value (a SOURCE
                // whose data flowed through this write), and a pending created
                // here belongs to THIS dest — creating it first let the commit
                // kill the dest's only materialization (the hsum stale-slot
                // miscompile: the deferred store vanished, the later consumer
                // reloaded never-written memory).
                self.sse_commit_dest_direct(dest_ptr, dst_static);
                if dst_home.is_none() {
                    let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
                    use crate::backend::state::SlotAddr;
                    if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                        self.state.resolve_slot_addr(dest_ptr.0)
                    {
                        if !deferred {
                            self.state.emit_fmt(format_args!(
                                "    movdqu %xmm0, {}",
                                self.slot_ref(slot.0)
                            ));
                        } else {
                            self.state.pending_vec_store = Some((dest_ptr.0, "xmm0", false));
                        }
                    }
                }
                return;
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
            // operand, never from the never-written home slot). WIDTH-EXACT:
            // a 128-bit fold re-issues a 16-byte load into the XMM view of
            // the requested register (a 32-byte read of a 16-byte object
            // could cross a page); the upper YMM half then holds nothing
            // this family reads, so `dirty_upper_ymm` stays clear exactly
            // like every other VEX.128 load.
            if let Some(pf) = self.state.pending_vec_memfold.clone() {
                if v.0 == pf.val {
                    self.state.pending_vec_memfold = None;
                    if self.state.pending_vec_store.map(|(_, r, _)| r) == Some(ymm) {
                        self.flush_pending_vec_store_impl();
                    }
                    // `ymm` is always "ymmN" (the reserved scratch pair);
                    // its 128-bit view is "xmmN".
                    let dst = if pf.width == 16 {
                        format!("x{}", &ymm[1..])
                    } else {
                        ymm.to_string()
                    };
                    self.state
                        .emit_fmt(format_args!("    {} {}, %{}", pf.mnemonic, pf.mem, dst));
                    // The re-issued load overwrites the register: every
                    // OTHER value's claim on that bank just died.
                    self.state.vec_evict_bank_except(ymm, v.0);
                    if pf.width != 16 {
                        self.state.dirty_upper_ymm = true;
                    }
                    self.state.vec_last_store_reg = false;
                    return;
                }
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
                        // The home->scratch copy overwrites `ymm`: every
                        // OTHER value's claim on that bank just died.
                        self.state.vec_evict_bank_except(ymm, v.0);
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
                    // The move-aside overwrites `ymm`: every OTHER value's
                    // claim on that bank just died.
                    self.state.vec_evict_bank_except(ymm, v.0);
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
                    self.state.vec_evict_bank_except(ymm, v.0);
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
                // The load overwrites `ymm` with `v`'s data: every OTHER
                // value's claim on that bank is now stale.
                self.state.vec_evict_bank_except(ymm, v.0);
                self.state.vec_last_store_reg = false;
                return;
            }
        }
        self.flush_pending_vec_store_impl();
        self.operand_to_reg(arg, "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu (%rax), %{}", ymm));
        if let Operand::Value(v) = arg {
            self.state.vec_evict_bank_except(ymm, v.0);
        }
        self.state.vec_last_store_reg = false;
    }
    pub(super) fn avx_load_arg(&mut self, arg: &Operand) {
        // 256-bit loads write the full YMM register: upper halves dirty.
        self.state.dirty_upper_ymm = true;
        self.avx_load_arg_to(arg, "ymm0");
    }

    /// Materialize a constant's raw object bits into `%rax`.
    ///
    /// Integer constants go through the generic [`Self::operand_to_reg`]
    /// (immediates, `xorl` zero idiom). FP constants must move through
    /// their BIT PATTERN: `operand_to_reg`'s catch-all zeroes the
    /// register (its contract is integer immediates), which silently
    /// turned an FP constant gather lane into +0.0 (BB-SLP VecPackF64x2
    /// with a `Const` lane). F64 bits always go `movabsq` (64-bit
    /// patterns); F32 bits fit `movl`, which zero-extends into `%rax`.
    /// The decimal/half/long-double consts are not lane types any vector
    /// family gathers and keep the previous (zeroing) behavior.
    fn stage_const_bits_to_rax(&mut self, c: &crate::ir::constants::IrConst) {
        use crate::ir::constants::IrConst;
        match c {
            IrConst::F64(v) => {
                let bits = v.to_bits() as i64;
                self.state
                    .emit_fmt(format_args!("    movabsq ${}, %rax", bits));
            }
            IrConst::F32(v) => {
                let bits = v.to_bits() as i32;
                self.state
                    .emit_fmt(format_args!("    movl ${}, %eax", bits));
            }
            other => {
                self.operand_to_reg(&Operand::Const(other.clone()), "rax");
            }
        }
    }

    /// Zero-splat fast path for the integer broadcast families that have no
    /// dedicated `VecZero*` intrinsic (the byte/halfword families).
    ///
    /// `Splat(Const(0))` through the generic broadcast staging costs 3
    /// instructions (`xorl`+`movd`+`vpbroadcastw`); a self-xor builds the
    /// all-zero vector in ONE — the exact lowering the `VecZero*` families
    /// use. The predicate is `is_all_zero_bits` (bit-exact, so an FP +0.0
    /// can never route here — these are integer-only call sites anyway).
    ///
    /// The 256-bit form writes the full register and marks the upper half
    /// dirty (the `VecZeroI64x4`/`VecZeroI32x8` convention: a later legacy
    /// SSE op on the same register needs the `vzeroupper` hygiene).
    fn try_zero_splat(&mut self, args: &[Operand], dest: &Option<Value>, avx: bool) -> bool {
        match &args.first() {
            Some(Operand::Const(c)) if c.is_all_zero_bits() => {}
            _ => return false,
        }
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if avx {
            self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
            self.state.dirty_upper_ymm = true;
        } else {
            self.state.emit("    pxor %xmm0, %xmm0");
        }
        if let Some(d) = dest {
            self.state.vector_values.insert(d.0);
            if avx {
                self.avx_store_dest(d);
            } else {
                self.sse_store_dest(d, "xmm0");
            }
        }
        true
    }

    /// All-ones splat fast path for the integer broadcast families.
    ///
    /// `Splat(Const(-1))` — BB-SLP seeds, the loop vectorizer's clamp
    /// masks, hand-written constants — materialized through the generic
    /// broadcast staging costs 3-4 instructions (movabs/mov + movd +
    /// punpck*/pshufd). A self-compare computes it in ONE: `pcmpeqd
    /// %xmm0, %xmm0` compares every dword of the register with itself,
    /// so all 32 dwords go ones — the all-ones pattern for EVERY
    /// sub-dword lane width, and both qwords of the 64-bit families.
    /// The 256-bit form (`vpcmpeqd %ymm0, %ymm0, %ymm0`) writes the
    /// full register, so it needs no dirty-upper discipline (exactly
    /// like the `vpxor` in `VecZeroI32x8`).
    ///
    /// FP broadcasts never route here: an FP `cmpeq` compares lane
    /// VALUES, not bits, and the all-ones bit pattern is a NaN pair —
    /// the callers only pass the integer families.
    fn try_all_ones_splat(&mut self, args: &[Operand], dest: &Option<Value>, avx: bool) -> bool {
        match &args.first() {
            Some(Operand::Const(c)) if c.to_i64() == Some(-1) => {}
            _ => return false,
        }
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        // Homed destination: build the ones directly IN the home register —
        // the self-compare's operands are the destination itself, so the
        // legacy two-operand form writes it in place and the VEX form names
        // it three times. This removes the xmm0 staging copy entirely (the
        // `pcmpeqd %xmm0, %xmm0; movdqa %xmm0, %xmmN` pair the showdown
        // shapes paid on every all-ones splat).
        if let Some(d) = dest {
            if let Some(&reg) = self.reg_assignments.get(&d.0) {
                if is_xmm_reg(reg) {
                    if avx {
                        let name = phys_reg_name_256(reg);
                        self.state
                            .emit_fmt(format_args!("    vpcmpeqd %{}, %{}, %{}", name, name, name));
                        self.note_vec_dest_in_home(d, &format!("%{}", name));
                    } else {
                        let name: &'static str = phys_reg_name(reg);
                        if self.avx2_enabled {
                            self.state.emit_fmt(format_args!(
                                "    vpcmpeqd %{}, %{}, %{}",
                                name, name, name
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    pcmpeqd %{}, %{}", name, name));
                        }
                        self.sse_commit_dest_direct(d, name);
                    }
                    self.state.vector_values.insert(d.0);
                    return true;
                }
            }
        }
        if avx {
            self.state.emit("    vpcmpeqd %ymm0, %ymm0, %ymm0");
        } else if self.avx2_enabled {
            self.state.emit("    vpcmpeqd %xmm0, %xmm0, %xmm0");
        } else {
            self.state.emit("    pcmpeqd %xmm0, %xmm0");
        }
        if let Some(d) = dest {
            self.state.vector_values.insert(d.0);
            if avx {
                self.avx_store_dest(d);
            } else {
                self.sse_store_dest(d, "xmm0");
            }
        }
        true
    }

    /// Store an extracted F64 lane (sitting in %xmm1) to its scalar
    /// dest. The dest is an ordinary F64: an 8-BYTE slot or XMM home.
    /// `sse_store_dest` is WRONG here — its `movdqu` writes 16 bytes
    /// and overflows the F64 slot into its neighbour (observed: the
    /// neighbour was the seed vector's own spill slot, and the vector
    /// store then re-read the clobbered lane — dq[0] got lane 3's
    /// value). `store_xmm0_fp_dest` has the right 8-byte `movsd`
    /// discipline; the XMM-home path goes direct from %xmm1.
    fn store_f64_lane_dest(&mut self, d: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&d.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name(reg);
                if name != "xmm1" {
                    self.state
                        .emit_fmt(format_args!("    movapd %xmm1, %{}", name));
                }
                self.state.reg_cache.invalidate_acc();
                self.note_inplace_compute(reg, d.0);
                return;
            }
        }
        self.state.emit("    movapd %xmm1, %xmm0");
        self.store_xmm0_fp_dest(d, IrType::F64);
    }

    /// XMM alias of a 256-bit register name ("ymm7" -> "xmm7"): the
    /// low 128 bits of a YMM register are directly readable as the
    /// same-numbered XMM register. Unknown names conservatively map to
    /// xmm0 (never constructed for live registers in practice).
    fn ymm_name_to_xmm(name: &str) -> &'static str {
        match name {
            "ymm1" => "xmm1",
            "ymm2" => "xmm2",
            "ymm3" => "xmm3",
            "ymm4" => "xmm4",
            "ymm5" => "xmm5",
            "ymm6" => "xmm6",
            "ymm7" => "xmm7",
            "ymm8" => "xmm8",
            "ymm9" => "xmm9",
            "ymm10" => "xmm10",
            "ymm11" => "xmm11",
            "ymm12" => "xmm12",
            "ymm13" => "xmm13",
            "ymm14" => "xmm14",
            "ymm15" => "xmm15",
            _ => "xmm0",
        }
    }

    /// Resolve the 128-bit half of a 256-bit vector operand into `%xmm1`.
    /// `half` selects which half (0 = low, 1 = high — the caller derives it
    /// from the lane index and the family's lanes-per-half; dword families
    /// have 4 lanes per half, qword families 2). Home register: the XMM alias
    /// (low half) or `vextracti128 $1` (high half). Slot-homed source: the
    /// relevant half loads DIRECTLY from the slot (`vmovdqu +0/+16`), never
    /// staging the full YMM. Returns false when nothing held the value
    /// (caller falls back to the pending-memfold materialization, which is
    /// the only legal producer of an unheld value).
    fn stage_256_half_to_xmm1(&mut self, arg: &Operand, half: usize) -> bool {
        let Operand::Value(v) = arg else {
            return false;
        };
        // A pending deferred store keeps the value in a register with a
        // stale slot — flush FIRST so the slot read below sees it.
        self.flush_pending_vec_store_impl();
        if let Some(&reg) = self.reg_assignments.get(&v.0) {
            if is_xmm_reg(reg) {
                let ymm = phys_reg_name_256(reg);
                let xmm = phys_reg_name(reg);
                if half == 1 {
                    self.state
                        .emit_fmt(format_args!("    vextracti128 $1, %{}, %xmm1", ymm));
                } else if xmm != "xmm1" {
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %xmm1", xmm));
                }
                return true;
            }
        }
        if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
            if half == 1 {
                self.state
                    .emit_fmt(format_args!("    vextracti128 $1, %{}, %xmm1", held));
            } else {
                let xmm = Self::ymm_name_to_xmm(held);
                if xmm != "xmm1" {
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %xmm1", xmm));
                }
            }
            return true;
        }
        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                let disp = if half == 1 { 16 } else { 0 };
                let mem = self.slot_ref(slot.0 + disp);
                self.state
                    .emit_fmt(format_args!("    vmovdqu {}, %xmm1", mem));
                return true;
            }
        }
        false
    }

    /// Store an extracted F32 lane (sitting in %xmm1) to its scalar dest.
    /// The dest is an ordinary F32: a 4-BYTE slot or XMM home. The same
    /// 16-byte-overflow discipline as `store_f64_lane_dest` (a `movdqu`
    /// store would clobber the 4-byte slot's neighbours); the XMM-home
    /// path goes direct from %xmm1 with `movaps`.
    fn store_f32_lane_dest(&mut self, d: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&d.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name(reg);
                if name != "xmm1" {
                    self.state
                        .emit_fmt(format_args!("    movaps %xmm1, %{}", name));
                }
                self.state.reg_cache.invalidate_acc();
                self.note_inplace_compute(reg, d.0);
                return;
            }
        }
        self.state.emit("    movaps %xmm1, %xmm0");
        self.store_xmm0_fp_dest(d, IrType::F32);
    }

    pub(super) fn avx_store_dest(&mut self, dest_ptr: &Value) {
        self.avx_store_dest_from(dest_ptr, "ymm0");
    }

    /// `avx_store_dest` with an explicit source register: the 256-bit
    /// result-homing discipline (RA home copy / slot store / deferred
    /// pending store) applied to a value that lives in `src` rather than
    /// `%ymm0` — e.g. the madd VLFOLD arm's 2XX form, which computes the
    /// result into the streamed operand's own (dying) register. Every
    /// downstream consumer of the pending/last-store machinery is
    /// register-name-generic, so the deferral and register-cache
    /// behaviour is identical to the %ymm0 spelling.
    fn avx_store_dest_from(&mut self, dest_ptr: &Value, src: &'static str) {
        // 256-bit store paths read the full YMM register; the earlier load
        // already dirtied it, but mark defensively (a broadcast-only body
        // could reach here through a register-copy path).
        self.state.dirty_upper_ymm = true;
        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name_256(reg);
                if name != src {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa %{}, %{}", src, name));
                }
                self.state.vec_claim_live_reg(dest_ptr.0, name);
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
                    self.state.emit_fmt(format_args!(
                        "    vmovdqu %{}, {}",
                        src,
                        self.slot_ref(slot.0)
                    ));
                } else {
                    self.state.pending_vec_store = Some((dest_ptr.0, src, true));
                }
                self.state.vec_last_store_slot = Some(slot.0);
                self.state.vec_last_store_val = Some(dest_ptr.0);
                self.state.vec_last_store_reg = true;
                self.state.vec_last_store_reg_name = Some(src);
                return;
            }
        }
        if !deferred {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    vmovdqu %{}, (%rax)", src));
        } else {
            self.state.pending_vec_store = Some((dest_ptr.0, src, true));
        }
        self.state.vec_last_store_val = Some(dest_ptr.0);
        self.state.vec_last_store_reg = true;
        self.state.vec_last_store_reg_name = Some(src);
    }

    /// Memory operand for a vector arg, unless the value is still provably in a
    /// register (last-stored/deferred or vec_live) — its slot contents may be
    /// stale under the deferred-store optimization. Returns None then, so
    /// the caller routes through the register cache instead of reading memory.
    fn vec_arg_mem(&self, arg: &Operand) -> Option<String> {
        // A VLFOLD-elided value's SOURCE operand is its only materialisation;
        // its home slot was never written, so hand back the recorded source
        // rather than the slot.  (The caller that consumes it is responsible
        // for clearing `pending_vec_memfold`; every such path either does so
        // or routes through `avx_load_arg_to`.)
        if let Some(mem) = self.memfold_operand(arg) {
            return Some(mem);
        }
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
        // A CONSTANT byte offset folds into the displacement: one SIB
        // operand, no %rcx staging. The ARX entry/exit state accesses and
        // every map load with a literal offset used to pay
        // `movl $16, %ecx; op (%rax,%rcx)` per access — the SIB disp form
        // (`vmovdqu 16(%rax)`) is one instruction. Only offsets that fit
        // the i32 disp field take this path; a huge constant still stages
        // through a register.
        if let Operand::Const(c) = off_arg {
            if let Some(v) = c.to_i64() {
                if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                    let (base, _) =
                        self.vec_load_addr_regs(base_arg, &Operand::Const(IrConst::I64(0)));
                    let d = disp + v;
                    let disp_str = if d == 0 {
                        String::new()
                    } else {
                        format!("{}", d)
                    };
                    return format!("{}(%{})", disp_str, base);
                }
            }
        }
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
    /// Register currently holding a 256-bit vector value for a VecStore:
    /// the value's allocated XMM home (YMM view) when register-assigned,
    /// then the block-local `vec_live_regs` entry, else `%ymm0` (the caller
    /// performs the canonical slot/peephole load into it).
    fn vec_store_source_256(&mut self, arg: &Operand) -> &'static str {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return phys_reg_name_256(reg);
                }
            }
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                return held;
            }
        }
        "ymm0"
    }

    /// XMM-view twin for the 128-bit store paths.
    fn vec_store_source_128(&mut self, arg: &Operand) -> &'static str {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return phys_reg_name(reg);
                }
            }
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                return held;
            }
        }
        "xmm0"
    }

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
        // VLFOLD: an eligible single-use 256-bit or 128-bit load emits
        // nothing; its adjacent consumer folds the source memory operand.
        if self.try_elide_vec_load(dest, op, args) {
            return;
        }
        // VLFOLD safety net: only the registered consumer or an intervening
        // pure vector load may follow an elided load; anything else
        // materialises it first.
        if let Some(pf) = self.state.pending_vec_memfold.clone() {
            // "Consumes" must mean the op can actually FOLD the elided
            // load's memory operand — an args match alone is not enough.
            // A lane extract (or any future non-folding consumer) naming
            // the value would otherwise read the register the allocator
            // reserved but the elided load never wrote. The consumer set
            // must ALSO match the fold's width: a 128-bit consumer cannot
            // fold a 256-bit memory operand (wrong family) and vice versa.
            use crate::backend::stack_layout::copy_coalescing as cc;
            // Immediate-shift consumers are width-irrelevant here: they can
            // never fold (VEX forms are register-only; see the ISA NOTE on
            // memfold_consumer_128), so a pending fold before one is always
            // materialised.
            let width_matches = if pf.width == 16 {
                cc::memfold_consumer_128(op).is_some() || cc::memfold_consumer_fma_128(op)
            } else {
                cc::memfold_consumer_256(op).is_some() || cc::memfold_consumer_madd_256(op)
            };
            let consumes = args
                .iter()
                .any(|a| matches!(a, Operand::Value(v) if v.0 == pf.val))
                && width_matches;
            if !consumes && !cc::is_pure_vec_load(op) {
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
            IntrinsicOp::FmaScalarF64Signed(np, na) => {
                // fma(-a, b, -c) & friends after the IR negation peel: the
                // sign flips ride the family selection (vfmsub / vfnmadd /
                // vfnmsub), the operand movement is the plain family's.
                // (np, na) == (false, false) is defensive — the peel emits
                // the plain intrinsic when both flags cancel.
                self.emit_scalar_fma_signed(
                    &args[0],
                    &args[1],
                    &args[2],
                    &dest.unwrap(),
                    IrType::F64,
                    *np,
                    *na,
                );
            }
            IntrinsicOp::FmaScalarF32Signed(np, na) => {
                self.emit_scalar_fma_signed(
                    &args[0],
                    &args[1],
                    &args[2],
                    &dest.unwrap(),
                    IrType::F32,
                    *np,
                    *na,
                );
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
                    if self.isa.fma {
                        self.state
                            .emit_fmt(format_args!("    vmovsd (%{}), %xmm1", a_name)); // xmm1 = A scalar
                        self.state.emit("    vunpcklpd %xmm1, %xmm1, %xmm1"); // xmm1 = {A, A}
                        self.state
                            .emit_fmt(format_args!("    vmovupd (%{}), %xmm0", c_name)); // xmm0 = {C[j], C[j+1]}
                        self.state
                            .emit_fmt(format_args!("    vfmadd231pd (%{}), %xmm1, %xmm0", b_name));
                        self.state
                            .emit_fmt(format_args!("    vmovupd %xmm0, (%{})", c_name)); // store back
                    } else {
                        // Legacy SSE2 (`-mno-avx` / `-march=x86-64`): separate
                        // mulpd + addpd (two roundings, i.e. exactly the
                        // scalar semantics under the default
                        // -ffp-contract=off).  B and C are loaded with
                        // `movupd` because neither 2×F64 row slice has a
                        // proven 16-byte alignment and the memory operand of
                        // `mulpd`/`addpd m128` faults on misalignment.
                        self.state
                            .emit_fmt(format_args!("    movsd (%{}), %xmm1", a_name));
                        self.state.emit("    unpcklpd %xmm1, %xmm1");
                        self.state
                            .emit_fmt(format_args!("    movupd (%{}), %xmm0", b_name));
                        self.state.emit("    mulpd %xmm1, %xmm0");
                        self.state
                            .emit_fmt(format_args!("    movupd (%{}), %xmm1", c_name));
                        self.state.emit("    addpd %xmm1, %xmm0");
                        self.state
                            .emit_fmt(format_args!("    movupd %xmm0, (%{})", c_name));
                    }

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
                    if self.isa.fma && self.isa.ymm {
                        self.state
                            .emit_fmt(format_args!("    vmovupd (%{}), %ymm0", c_name));
                        self.state
                            .emit_fmt(format_args!("    vfmadd231pd (%{}), %ymm1, %ymm0", b_name));
                        self.state
                            .emit_fmt(format_args!("    vmovupd %ymm0, (%{})", c_name));
                    } else {
                        // Legacy SSE2: two 2-lane mul+add pairs per group.
                        // The broadcast factor {A, A} lives in %xmm1
                        // (BroadcastLoadF64 legacy form) and must survive
                        // both halves and every iteration, the product needs
                        // a register (%xmm0) and so does the C addend: SSE2
                        // has no unaligned memory-source arithmetic and
                        // neither slice is provably 16-byte aligned.  The
                        // third register is %xmm2, which the allocator
                        // withholds from any function containing this
                        // intrinsic (regalloc_helpers `clobbers_xmm2`, the
                        // same contract as VecMulI64x2 / Pblendvb128).
                        for half in [0u32, 16] {
                            self.state
                                .emit_fmt(format_args!("    movupd {}(%{}), %xmm0", half, b_name));
                            self.state.emit("    mulpd %xmm1, %xmm0");
                            self.state
                                .emit_fmt(format_args!("    movupd {}(%{}), %xmm2", half, c_name));
                            self.state.emit("    addpd %xmm2, %xmm0");
                            self.state
                                .emit_fmt(format_args!("    movupd %xmm0, {}(%{})", half, c_name));
                        }
                    }
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
                if self.isa.fma && self.isa.ymm {
                    self.state.emit("    vmovsd (%rcx), %xmm1");
                    self.state.emit("    vbroadcastsd %xmm1, %ymm1");
                } else {
                    // Legacy SSE2 consumer (FmaF64x2Hoisted) reads {A, A}
                    // from %xmm1.
                    self.state.emit("    movsd (%rcx), %xmm1");
                    self.state.emit("    unpcklpd %xmm1, %xmm1");
                }
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

                // Intrinsic-scratch pool (the caller-saved GP registers
                // the emitter may clobber around intrinsics; see the
                // FmaF64x4 scratch contract above). A reload fallback must
                // never land on a register already holding one of this
                // instruction's own operands: the C row base homed in %rsi
                // while a spilled j-offset reloaded into the fixed %rsi
                // scratch emitted `vmovupd (%rsi,%rsi)` — address 2*offset —
                // and miscompiled remainder-shaped FMA loops (segfault at
                // 2*j). Probe every operand's home up front so each reload
                // avoids all of them, not just the names resolved earlier.
                let c_home = self.operand_reg(&args[0]).map(super::emit::phys_reg_name);
                let b_home = self.operand_reg(&args[1]).map(super::emit::phys_reg_name);
                let off_home = self
                    .operand_reg(&off_operand)
                    .map(super::emit::phys_reg_name);
                let homes = [c_home, b_home, off_home];
                let pool = ["rax", "rcx", "rdx", "rsi", "rdi"];
                let home_taken = |r: &str| homes.iter().any(|h| *h == Some(r));
                let c_name = match c_home {
                    Some(name) => name,
                    None => {
                        let r = pool
                            .iter()
                            .find(|r| !home_taken(r))
                            .copied()
                            .expect("five-reg scratch pool, at most two operand homes");
                        self.operand_to_reg(&args[0], r);
                        r
                    }
                };
                let b_name = match b_home {
                    Some(name) => name,
                    None => {
                        let r = pool
                            .iter()
                            .find(|r| !home_taken(r) && **r != c_name)
                            .copied()
                            .expect("five-reg scratch pool, at most two operand homes");
                        self.operand_to_reg(&args[1], r);
                        r
                    }
                };
                let off_name = match off_home {
                    Some(name) => name,
                    None => {
                        let r = pool
                            .iter()
                            .find(|r| !home_taken(r) && **r != c_name && **r != b_name)
                            .copied()
                            .expect("five-reg scratch pool, at most two operand homes");
                        self.operand_to_reg(&off_operand, r);
                        r
                    }
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
                // Keep the scalar result in the SSE domain: an XMM-homed
                // destination receives `movapd %xmm0, %xmmN`, a stack-slot
                // destination a direct `movsd`, never a GPR round trip.
                if let Some(d) = dest {
                    self.store_xmm_to(d, "xmm0", IrType::F64);
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
                if let Some(d) = dest {
                    self.store_xmm_to(d, "xmm0", IrType::F64);
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
                            // A 256-bit load writes the full YMM: the upper
                            // halves are dirty and the function epilogue owes
                            // a `vzeroupper` (this arm used to skip the flag,
                            // so a loop whose ONLY 256-bit op was a homed load
                            // left the AVX-SSE transition penalty in place).
                            self.state.dirty_upper_ymm = true;
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
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
            IntrinsicOp::VecLoadWidenI32ToI64x2 => {
                // Two I32 lanes sign-extended to two I64 lanes (the SSE2
                // half of the widening `long += int[i]` reduction; the AVX2
                // form is `vpmovsxdq`).  SSE4.1's `pmovsxdq` is not in the
                // baseline ISA, so build the extension from SSE2 primitives:
                //   movq      mem, %xmm0     ; xmm0 = {a, b, 0, 0} (I32 lanes)
                //   movdqa    %xmm0, %xmm1
                //   psrad     $31, %xmm1     ; xmm1 = {sa, sb, 0, 0} (sign words)
                //   punpckldq %xmm1, %xmm0   ; xmm0 = {a, sa, b, sb} = {sext a, sext b}
                // Before this arm existed the op reached the emitter only via
                // LCCC_FORCE_SSE2 and ICEd ("unhandled intrinsic op"); with the
                // ISA gate selecting the 128-bit vectoriser for -mno-avx /
                // baseline TUs it is a mainstream path.
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                self.state.emit_fmt(format_args!("    movq {}, %xmm0", mem));
                self.state.emit("    movdqa %xmm0, %xmm1");
                self.state.emit("    psrad $31, %xmm1");
                self.state.emit("    punpckldq %xmm1, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecLoadI64x2 => {
                // Load two I64 lanes (movdqu). Same addressing as VecLoadF64x2.
                // An optional third argument is a constant displacement
                // (vec_interleave interleave slices), folded into the SIB
                // operand.
                // A homed destination loads straight into its home: the FP
                // twin (VecLoadF64x2) has always done this, while the integer
                // form staged through %xmm0 and paid a `movdqa` per load.
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movdqu {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.sse_store_dest(d, "xmm0");
                    }
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
                            self.state.vec_claim_live_reg(d.0, name);
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
                // Load straight into the destination's YMM home when it has
                // one: the generic path stages through %ymm0 and pays a
                // `vmovdqa` per iteration.  The FP twin (VecLoadF64x4) has
                // always done this; the integer form had not, which cost one
                // register copy in every integer map/reduction loop.
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, name));
                            self.state.dirty_upper_ymm = true;
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    // Defer-aware store; reuse register-allocated base/offset.
                    // Commit a pending deferred %ymm0 result before clobbering
                    // the scratch register (see VecLoadF64x4).  An optional
                    // third argument is a constant displacement (vec_interleave
                    // interleave slices), folded into the SIB operand.
                    self.flush_pending_vec_store_impl();
                    self.state
                        .emit_fmt(format_args!("    vmovdqu {}, %ymm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecLoadI32x4 => {
                // An optional third argument is a constant displacement
                // (vec_interleave interleave slices), folded into the SIB
                // operand.
                // A homed destination loads straight into its home (same
                // pattern as VecLoadF64x2/VecLoadI64x2): staging through
                // %xmm0 cost a `movdqa` per load in every integer SSE loop.
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movdqu {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }

            IntrinsicOp::VecLoadI8x32 => {
                // Byte-lane map stream load: identical to VecLoadI32x8 —
                // 32 contiguous bytes via one vmovdqu %ymm.  A homed
                // destination loads straight into its register.
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            // %ymm0 is NOT clobbered: a pending deferred
                            // value survives untouched for its consumer.
                            let mem = self.vec_mem_operand(
                                &args[0],
                                &args[1],
                                Self::vec_disp_arg(args, 2),
                            );
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, name));
                            self.state.dirty_upper_ymm = true;
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            return;
                        }
                    }
                }
                self.flush_pending_vec_store_impl();
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                self.state
                    .emit_fmt(format_args!("    vmovdqu {}, %ymm0", mem));
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
                }
            }

            IntrinsicOp::VecLoadI16x16 => {
                // Halfword-lane stream load: 32 contiguous bytes via one
                // vmovdqu %ymm, identical in shape to VecLoadI8x32. A
                // homed destination loads straight into its register.
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            // %ymm0 is NOT clobbered: a pending deferred
                            // value survives untouched for its consumer.
                            let mem = self.vec_mem_operand(
                                &args[0],
                                &args[1],
                                Self::vec_disp_arg(args, 2),
                            );
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, name));
                            self.state.dirty_upper_ymm = true;
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            return;
                        }
                    }
                }
                self.flush_pending_vec_store_impl();
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                self.state
                    .emit_fmt(format_args!("    vmovdqu {}, %ymm0", mem));
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
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
                // An optional fourth argument is a constant displacement
                // (vec_interleave interleave slices), folded into the SIB
                // operand.
                let mem = self.vec_mem_operand(&args[1], &args[2], Self::vec_disp_arg(args, 3));
                self.state
                    .emit_fmt(format_args!("    vmovdqu {}, %xmm0", mem));
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
                            self.state.vec_claim_live_reg(d.0, dst);
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
                            self.state.vec_claim_live_reg(d.0, dst);
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
                            self.state.vec_claim_live_reg(d.0, dst);
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
                // Register-home source: store straight from the assigned XMM
                // register (mirrors VecStoreI32x4). The old form hardcoded
                // %xmm0 — correct only for deferred-slot chains; with an
                // RA-homed source (the BB-SLP I64x2 families home load/sub
                // results) it stored stale %xmm0 contents (miscompile:
                // simd_vecreg_new_ops' reference w64 subtraction).
                let src = self.vec_store_source_128(&args[0]);
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "movdqu", src);
                    return;
                }
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
            IntrinsicOp::VecBroadcastI64x2 => {
                // All-ones splat: one self-compare instead of the staged
                // movq/movd/unpcklpd chain.
                if !self.try_all_ones_splat(args, dest, false) {
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
                                self.state.out.emit_instr_rbp_reg(
                                    "    movq",
                                    slot.0 as i64,
                                    "xmm0",
                                );
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
            }
            IntrinsicOp::VecLoadI64x4 => {
                // 4×I64/U64 lanes (vmovdqu ymm) — the BB-SLP 32-byte copy
                // load. Mirrors VecLoadF64x4's home-direct discipline: an
                // XMM-homed destination loads straight into its YMM home.
                // (This arm used to be a no-op placeholder shared with
                // VecZeroI64x4; any IR reaching it silently did nothing.)
                let disp = Self::vec_disp_arg(args, 2);
                let mem = self.vec_mem_operand(&args[0], &args[1], disp);
                self.state.dirty_upper_ymm = true;
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    vmovdqu {}, %ymm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecZeroI64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name_256(reg);
                        // VEX 3-operand form: `vpxor %r, %r` (2 operands)
                        // does not exist — the VEX encoding always carries
                        // dst, src1, src2. The 2-op form this arm used to
                        // emit was never assembled (no AVX2-gated test
                        // reached a VecZeroI64x4 before the BB-SLP
                        // constant-store seeds unlocked it).
                        self.state
                            .emit_fmt(format_args!("    vpxor %{}, %{}, %{}", name, name, name));
                        self.state.dirty_upper_ymm = true;
                        self.state.vec_claim_live_reg(d.0, name);
                    } else {
                        self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
                        self.state.dirty_upper_ymm = true;
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecStoreI64x4 => {
                // 4×I64/U64 store (vmovdqu ymm → mem): register-home source
                // stores straight from the assigned YMM register, exactly
                // like VecStoreI32x8. Without the home path an RA-homed
                // source would store stale %ymm0 contents.
                let src = self.vec_store_source_256(&args[0]);
                if src != "ymm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "vmovdqu", src);
                    return;
                }
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
            IntrinsicOp::VecSubI64x4 => {
                // Lane-wise 4×I64 subtract (vpsubq): non-commutative, so a
                // folded memory operand is only legal in the src2 position.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpsubq", false);
                }
            }
            IntrinsicOp::VecAndI64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpand", true);
                }
            }
            IntrinsicOp::VecOrI64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpor", true);
                }
            }
            IntrinsicOp::VecXorI64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpxor", true);
                }
            }
            IntrinsicOp::VecBroadcastI64x4 => {
                // All-ones splat: one vpcmpeqd instead of the staged chain.
                if !self.try_all_ones_splat(args, dest, true) {
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
                                        "    vmovdqa %{}, %xmm0",
                                        phys_reg_name_256(reg)
                                    ));
                                }
                            } else if let Some(slot) = self.state.get_slot(v.0) {
                                self.state.out.emit_instr_rbp_reg(
                                    "    movq",
                                    slot.0 as i64,
                                    "xmm0",
                                );
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
                    self.state.emit("    vpbroadcastq %xmm0, %ymm0");
                    self.state.dirty_upper_ymm = true;
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecAndI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pand");
                }
            }
            IntrinsicOp::VecOrI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "por");
                }
            }
            IntrinsicOp::VecXorI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pxor");
                }
            }
            IntrinsicOp::VecLoadI16x8 | IntrinsicOp::VecLoadI8x16 => {
                // 8×I16 / 16×I8 lanes (movdqu xmm) — byte/halfword copy
                // chains. Same home-direct discipline as VecLoadI64x2.
                let disp = Self::vec_disp_arg(args, 2);
                let mem = self.vec_mem_operand(&args[0], &args[1], disp);
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movdqu {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecStoreI16x8 | IntrinsicOp::VecStoreI8x16 => {
                // Register-home source: store straight from the assigned XMM
                // register (mirrors VecStoreI32x4).
                let src = self.vec_store_source_128(&args[0]);
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "movdqu", src);
                    return;
                }
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
            IntrinsicOp::VecAndI8x16 | IntrinsicOp::VecAndI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pand");
                }
            }
            IntrinsicOp::VecOrI8x16 | IntrinsicOp::VecOrI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "por");
                }
            }
            IntrinsicOp::VecXorI8x16 | IntrinsicOp::VecXorI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pxor");
                }
            }
            IntrinsicOp::VecPackI64x2 | IntrinsicOp::VecPackF64x2 => {
                // Gather two GPR-domain scalars into one 2-lane vector:
                //   movq lo, %xmm0
                //   movq hi, %rax ; movq %rax, %xmm1
                //   punpcklqdq %xmm1, %xmm0
                // Arguments are scalars, so the XMM scratch pair is safe.
                // An XMM-homed FP source loads directly (movq xmm→xmm).
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let load_lane_to_xmm0 = |this: &mut Self, arg: &Operand| {
                    if let Operand::Value(v) = arg {
                        if let Some(&reg) = this.reg_assignments.get(&v.0) {
                            if is_xmm_reg(reg) {
                                let name = phys_reg_name(reg);
                                this.state
                                    .emit_fmt(format_args!("    movq %{}, %xmm0", name));
                                return;
                            }
                        }
                        if let Some(slot) = this.state.get_slot(v.0) {
                            this.state
                                .out
                                .emit_instr_rbp_reg("    movq", slot.0 as i64, "xmm0");
                            return;
                        }
                    }
                    // Constant lanes: integer immediates through the
                    // generic path, FP constants through their BIT PATTERN
                    // (operand_to_reg's catch-all would zero the lane —
                    // the VecPackF64x2 const-lane miscompile).
                    if let Operand::Const(c) = arg {
                        this.stage_const_bits_to_rax(c);
                    } else {
                        this.operand_to_reg(arg, "rax");
                    }
                    this.state.emit("    movq %rax, %xmm0");
                };
                load_lane_to_xmm0(self, &args[0]);
                // Second lane: stage through %rax/%xmm1 (never clobbers
                // xmm0). Constant lanes take the bit-pattern path (see
                // lane 0).
                if let Operand::Value(v) = &args[1] {
                    if let Some(&reg) = self.reg_assignments.get(&v.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movq %{}, %xmm1", name));
                            self.state.emit("    punpcklqdq %xmm1, %xmm0");
                            if let Some(d) = dest {
                                self.state.vector_values.insert(d.0);
                                self.sse_store_dest(d, "xmm0");
                            }
                            return;
                        }
                    }
                }
                if let Operand::Const(c) = &args[1] {
                    self.stage_const_bits_to_rax(c);
                } else {
                    self.operand_to_reg(&args[1], "rax");
                }
                self.state.emit("    movq %rax, %xmm1");
                self.state.emit("    punpcklqdq %xmm1, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecLoadI32x4Pair => {
                // HALF-WIDE dword-pair load: two consecutive dwords into
                // lanes 0/1, upper 64 bits ZEROED (MOVQ xmm, m64 clears
                // the high half in both the legacy and VEX encodings).
                // Home-direct discipline identical to VecLoadI64x2. The
                // load is NEVER elided into a consumer's r/m slot (the
                // VLFOLD match table deliberately does not list it — an
                // 8-byte fold would need width-8 pending machinery, and
                // the half-wide shapes want the explicit pair load).
                let mem = self.vec_mem_operand(&args[0], &args[1], Self::vec_disp_arg(args, 2));
                let mnemonic = if self.avx2_enabled { "vmovq" } else { "movq" };
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    {} {}, %{}", mnemonic, mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_claim_live_reg(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    {} {}, %xmm0", mnemonic, mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecStoreI32x4Pair => {
                // HALF-WIDE dword-pair store: lanes 0/1 of the XMM source
                // as 8 bytes (MOVQ xmm, m64). Mirrors VecStoreI64x2's
                // register-home discipline exactly — the source is the
                // I32x4-family web's value, read through the shared
                // vec_store_source_128 path.
                let src = self.vec_store_source_128(&args[0]);
                let mnemonic = if self.avx2_enabled { "vmovq" } else { "movq" };
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, mnemonic, src);
                    return;
                }
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, %xmm0",
                                    mnemonic,
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
                self.emit_vec_store_addr(args, dest_ptr, mnemonic, "xmm0");
            }
            IntrinsicOp::VecPackI32x4Pair => {
                // HALF-WIDE dword-pair gather: two scalar dwords into
                // lanes 0/1 with lanes 2/3 ZERO (movd clears bits 32..127,
                // punpckldq interleaves the low dwords of two
                // zero-uppered registers — [a,0,0,0] ⊕ [b,0,0,0] →
                // [a,b,0,0]). SSE2-exact: no pinsrd dependency.
                //   movd lo, %xmm0
                //   movd hi, %xmm1 ; punpckldq %xmm1, %xmm0
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let load_lane = |this: &mut Self, arg: &Operand, xmm: &'static str| {
                    if let Operand::Value(v) = arg {
                        if let Some(&reg) = this.reg_assignments.get(&v.0) {
                            if !is_xmm_reg(reg) {
                                this.state.emit_fmt(format_args!(
                                    "    movd %{}, %{}",
                                    phys_reg_name(reg),
                                    xmm
                                ));
                                return;
                            }
                        }
                        if let Some(slot) = this.state.get_slot(v.0) {
                            this.state
                                .out
                                .emit_instr_rbp_reg("    movd", slot.0 as i64, xmm);
                            return;
                        }
                    }
                    this.operand_to_reg(arg, "rax");
                    this.state.emit_fmt(format_args!("    movd %rax, %{}", xmm));
                };
                load_lane(self, &args[0], "xmm0");
                load_lane(self, &args[1], "xmm1");
                self.state.emit("    punpckldq %xmm1, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecExtractLaneI64x2 => {
                // Lane → GPR scalar. Lane 0: movq; lane 1: pshufd $0x0E
                // first (SSE2 — no pextrq dependency). Scratch %xmm1 only.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.sse_load_arg(&args[0], "xmm1");
                if lane & 1 != 0 {
                    self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                }
                self.state.emit("    movq %xmm1, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecExtractLaneF64x2 => {
                // Lane → XMM scalar. Lane 0: identity move; lane 1:
                // pshufd $0x0E. The dest is an ordinary F64 SSE value.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let mut handled_fast = false;
                // REGISTER-SOURCED fast path (the nbody d² chain): a
                // homed or live XMM source extracts with ZERO staging —
                // lane 0 IS the register's low double (a direct
                // `movsd %src, DST` stores it), lane 1 is one pshufd.
                // The generic path below pays movdqa+movapd copies and a
                // slot round trip that sit directly on the d² dependency
                // chain of every vectorized pair loop.
                if let Operand::Value(v) = &args[0] {
                    let src_reg = self
                        .reg_assignments
                        .get(&v.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(phys_reg_name)
                        .or_else(|| {
                            self.state
                                .vec_live_regs
                                .get(&v.0)
                                .copied()
                                .filter(|n| n.starts_with("xmm"))
                        });
                    if let Some(src_name) = src_reg {
                        if let Some(d) = dest {
                            let dst_home = self.dest_xmm_home_name(d);
                            if lane & 1 == 0 {
                                match dst_home {
                                    Some(h) if h != src_name => {
                                        self.state.emit_fmt(format_args!(
                                            "    movapd %{}, %{}",
                                            src_name, h
                                        ));
                                        self.note_inplace_compute(
                                            self.reg_assignments.get(&d.0).copied().unwrap(),
                                            d.0,
                                        );
                                        self.state.reg_cache.invalidate_acc();
                                    }
                                    Some(_) => {
                                        self.state.reg_cache.invalidate_acc();
                                    }
                                    None => {
                                        if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                            self.state.resolve_slot_addr(d.0)
                                        {
                                            self.state.emit_fmt(format_args!(
                                                "    movsd %{}, {}",
                                                src_name,
                                                self.slot_ref(slot.0)
                                            ));
                                            self.state.sse_last_store_slot = Some(slot.0);
                                        } else {
                                            self.state.emit_fmt(format_args!(
                                                "    movapd %{}, %xmm0",
                                                src_name
                                            ));
                                            self.store_xmm0_fp_dest(d, IrType::F64);
                                        }
                                    }
                                }
                            } else {
                                match dst_home {
                                    Some(h) => {
                                        self.state.emit_fmt(format_args!(
                                            "    pshufd $0x0E, %{}, %{}",
                                            src_name, h
                                        ));
                                        self.note_inplace_compute(
                                            self.reg_assignments.get(&d.0).copied().unwrap(),
                                            d.0,
                                        );
                                        self.state.reg_cache.invalidate_acc();
                                    }
                                    None => {
                                        self.state.emit_fmt(format_args!(
                                            "    pshufd $0x0E, %{}, %xmm0",
                                            src_name
                                        ));
                                        self.store_xmm0_fp_dest(d, IrType::F64);
                                    }
                                }
                            }
                            handled_fast = true;
                        }
                    }
                }
                if !handled_fast {
                    self.sse_load_arg(&args[0], "xmm1");
                    if lane & 1 != 0 {
                        self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                    }
                    if let Some(d) = dest {
                        self.store_f64_lane_dest(d);
                    }
                }
            }
            IntrinsicOp::VecExtractLaneI64x4 => {
                // Lane → GPR scalar from a 256-bit source. The relevant
                // 128-bit half lands in %xmm1 (XMM alias of the YMM home,
                // vextracti128 for the high half, or a direct half-slot
                // load); then the I64x2 lane finish (odd lanes via
                // pshufd). Scratch: %xmm1 only — an XMM home is read,
                // never written.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if self.stage_256_half_to_xmm1(&args[0], (lane / 2) as usize) {
                    if lane & 1 != 0 {
                        self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                    }
                    self.state.emit("    movq %xmm1, %rax");
                    if let Some(d) = dest {
                        self.store_rax_to(d);
                    }
                } else {
                    // No home/live register/slot held the value: the def
                    // was elided by VLFOLD. Materialize the pending load
                    // and retry once.
                    self.materialize_pending_memfold();
                    if self.stage_256_half_to_xmm1(&args[0], (lane / 2) as usize) {
                        if lane & 1 != 0 {
                            self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                        }
                        self.state.emit("    movq %xmm1, %rax");
                        if let Some(d) = dest {
                            self.store_rax_to(d);
                        }
                    } else {
                        // Backend contract violation: every consumed
                        // vector value is register-homed, live-held, or
                        // slot-homed (VLFOLD materialization covers the
                        // elided-load case). Emitting nothing would be a
                        // SILENT MISCOMPILE — the dest would keep stale
                        // bits. Refuse loudly instead.
                        panic!(
                            "VecExtractLaneI64x4: source has no register/live/slot home (lane {lane})"
                        );
                    }
                }
            }
            IntrinsicOp::VecExtractLaneF64x4 => {
                // Lane → XMM scalar from a 256-bit source: the same
                // half-selection, then the F64x2 finish. The dest is an
                // ordinary F64 SSE value.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if self.stage_256_half_to_xmm1(&args[0], (lane / 2) as usize) {
                    if lane & 1 != 0 {
                        self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                    }
                    if let Some(d) = dest {
                        self.store_f64_lane_dest(d);
                    }
                } else {
                    self.materialize_pending_memfold();
                    if self.stage_256_half_to_xmm1(&args[0], (lane / 2) as usize) {
                        if lane & 1 != 0 {
                            self.state.emit("    pshufd $0x0E, %xmm1, %xmm1");
                        }
                        if let Some(d) = dest {
                            self.store_f64_lane_dest(d);
                        }
                    } else {
                        panic!(
                            "VecExtractLaneF64x4: source has no register/live/slot home (lane {lane})"
                        );
                    }
                }
            }
            IntrinsicOp::VecExtractLaneF32x4 => {
                // Lane → F32 scalar from a 128-bit source: stage into
                // %xmm1, select the dword (lane 0 needs no shuffle), and
                // store with the 4-byte movss discipline.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.sse_load_arg(&args[0], "xmm1");
                if lane != 0 {
                    self.state
                        .emit_fmt(format_args!("    pshufd ${}, %xmm1, %xmm1", lane & 3));
                }
                if let Some(d) = dest {
                    self.store_f32_lane_dest(d);
                }
            }
            IntrinsicOp::VecExtractLaneF32x8 => {
                // Lane → F32 scalar from a 256-bit source: half staging
                // (4 dwords per half), in-half dword select, movss store.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if self.stage_256_half_to_xmm1(&args[0], (lane / 4) as usize) {
                    if lane & 3 != 0 {
                        self.state
                            .emit_fmt(format_args!("    pshufd ${}, %xmm1, %xmm1", lane & 3));
                    }
                    if let Some(d) = dest {
                        self.store_f32_lane_dest(d);
                    }
                } else {
                    self.materialize_pending_memfold();
                    if self.stage_256_half_to_xmm1(&args[0], (lane / 4) as usize) {
                        if lane & 3 != 0 {
                            self.state
                                .emit_fmt(format_args!("    pshufd ${}, %xmm1, %xmm1", lane & 3));
                        }
                        if let Some(d) = dest {
                            self.store_f32_lane_dest(d);
                        }
                    } else {
                        panic!(
                            "VecExtractLaneF32x8: source has no register/live/slot home (lane {lane})"
                        );
                    }
                }
            }
            IntrinsicOp::VecExtractLaneI32x8 => {
                // Lane → GPR scalar from a 256-bit source: half staging
                // (4 dwords per half), then the I32x4 finish — pextrd on
                // SSE4.1 hosts, pshufd+movd on the SSE2 baseline.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                let in_half = lane & 3;
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let mut finish = |this: &mut Self| {
                    if this.isa.sse41 {
                        this.state
                            .emit_fmt(format_args!("    pextrd ${}, %xmm1, %eax", in_half));
                    } else {
                        if in_half != 0 {
                            this.state
                                .emit_fmt(format_args!("    pshufd ${}, %xmm1, %xmm1", in_half));
                        }
                        this.state.emit("    movd %xmm1, %eax");
                    }
                    if let Some(d) = &dest {
                        this.store_rax_to(d);
                    }
                };
                if self.stage_256_half_to_xmm1(&args[0], (lane / 4) as usize) {
                    finish(self);
                } else {
                    self.materialize_pending_memfold();
                    if self.stage_256_half_to_xmm1(&args[0], (lane / 4) as usize) {
                        finish(self);
                    } else {
                        panic!(
                            "VecExtractLaneI32x8: source has no register/live/slot home (lane {lane})"
                        );
                    }
                }
            }
            IntrinsicOp::VecExtractLaneI16x8 => {
                // Lane → GPR scalar from a 128-bit halfword source:
                // `pextrw` (SSE2 baseline — no SSE4.1 dependency), then
                // the canonical GPR-value store.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.sse_load_arg(&args[0], "xmm1");
                self.state
                    .emit_fmt(format_args!("    pextrw ${}, %xmm1, %eax", lane & 7));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecExtractLaneI16x16 => {
                // Lane → GPR scalar from a 256-bit halfword source: half
                // staging (8 words per half) then pextrw of the in-half
                // lane.
                let lane = match &args[1] {
                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                    _ => 0,
                };
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if self.stage_256_half_to_xmm1(&args[0], (lane / 8) as usize) {
                    self.state
                        .emit_fmt(format_args!("    pextrw ${}, %xmm1, %eax", lane & 7));
                    if let Some(d) = dest {
                        self.store_rax_to(d);
                    }
                } else {
                    self.materialize_pending_memfold();
                    if self.stage_256_half_to_xmm1(&args[0], (lane / 8) as usize) {
                        self.state
                            .emit_fmt(format_args!("    pextrw ${}, %xmm1, %eax", lane & 7));
                        if let Some(d) = dest {
                            self.store_rax_to(d);
                        }
                    } else {
                        panic!(
                            "VecExtractLaneI16x16: source has no register/live/slot home (lane {lane})"
                        );
                    }
                }
            }
            IntrinsicOp::VecAddI64x4 => {
                // 4×I64 lane add (`vpaddq`): a plain commutative 3-operand
                // VEX binary — lane-agnostic, so the shared emitter's
                // home/memfold/all-homed paths apply unchanged.  Used by the
                // byte-predicate counting reduction to accumulate the four
                // `vpsadbw` partial sums.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpaddq", true);
                }
            }
            IntrinsicOp::VecSadbwU8x32 => {
                // Sum of absolute byte differences (`vpsadbw`): commutative
                // 3-operand VEX binary over the byte lanes, producing four
                // u64 group sums per YMM.  The counting reduction pairs it
                // with an all-zero vector, so 0/1 byte lanes become exact
                // per-group counts.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpsadbw", true);
                }
            }
            IntrinsicOp::VecMaddubsU8x32 => {
                // Unsigned×signed byte pair multiply-add (`vpmaddubsw`):
                // NON-commutative 3-operand VEX binary.  AT&T
                // `vpmaddubsw r/m, reg, dst` takes the SIGNED weights in
                // the r/m (first textual) slot = IR args[1], the unsigned
                // byte stream in the register slot = IR args[0]; operand
                // order is preserved by `commutative = false` so the two
                // roles can never swap (a swap would feed the byte stream
                // to the signed multiplier and the weights to the unsigned
                // one — a silent miscompile class).
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaddubsw", false);
                }
            }
            IntrinsicOp::VecMaddwdI16x16 => {
                // Word pair multiply-add (`vpmaddwd`): the Adler-32 epic
                // sums the maddubs word products with an all-ones table
                // into dword lanes.  Order-preserving (commutative=false)
                // for the same discipline as maddubs.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaddwd", false);
                }
            }
            IntrinsicOp::VecConstI8x32 => {
                // 32-byte .rodata constant vector (Adler-32 epic: the
                // `[32, 31, ..., 1]` weight table, the i16 ones table, the
                // lane-0 seed mask).  The byte pattern is deduplicated
                // through the shared `.LCVEC` const pool (the v6 FP-neg
                // infrastructure), then materialised like VecZeroI64x4:
                // straight into a register home when the web allocated one,
                // otherwise through the %ymm0 scratch + slot store.
                // Args MUST be exactly 32 const bytes — anything else is an
                // IR-construction bug, never a silent default.
                assert!(
                    args.len() == 32,
                    "VecConstI8x32: expected 32 const byte args, got {}",
                    args.len()
                );
                let mut bytes = [0u8; 32];
                for (i, slot) in bytes.iter_mut().enumerate() {
                    let byte = match &args[i] {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    };
                    let Some(byte) = byte else {
                        panic!(
                            "VecConstI8x32: arg {} is not a constant (IR construction bug)",
                            i
                        );
                    };
                    assert!(
                        (-128..=127).contains(&byte),
                        "VecConstI8x32: arg {} = {} out of byte range",
                        i,
                        byte
                    );
                    *slot = byte as u8;
                }
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let label = self.state.get_vec_const_label(&bytes);
                let mem = format!("{}(%rip)", label);
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name_256(reg);
                        self.state
                            .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, name));
                        self.state.dirty_upper_ymm = true;
                        self.state.vec_claim_live_reg(d.0, name);
                        self.state.vec_last_store_val = Some(d.0);
                        self.state.vec_last_store_reg = true;
                        self.state.vec_last_store_reg_name = Some(name);
                    } else {
                        self.state
                            .emit_fmt(format_args!("    vmovdqu {}, %ymm0", mem));
                        self.state.dirty_upper_ymm = true;
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecHorizontalAddI64x4 => {
                // 4×I64 YMM → one I64 scalar: extract the high 128 bits,
                // fold the four lanes down pairwise, land the result in
                // %rax.  `vpsadbw` accumulators are u64, so the counting
                // reduction's totals (bounded by the element count) stay
                // exact; the movq is a plain 64-bit move of lane 0.
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.avx_load_arg(&args[0]);
                self.state.emit("    vextracti128 $1, %ymm0, %ymm1");
                self.state.emit("    vpaddq %ymm1, %ymm0, %ymm0");
                self.state.emit("    vpshufd $0xEE, %xmm0, %xmm1");
                self.state.emit("    vpaddq %xmm1, %xmm0, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
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
            IntrinsicOp::VecXorF32x8 => {
                if let Some(d) = dest {
                    // Bitwise lane XOR — commutative, pure, and the exact
                    // packed spelling of FP sign-bit manipulation idioms
                    // (see the IR enum doc on `VecXorF32x8`).
                    if !self.try_emit_fpxor_sign_mask(d, args, 4, true, "vxorps") {
                        self.emit_avx_binary_256(d, args, "vxorps", true);
                    }
                }
            }
            IntrinsicOp::VecXorF64x4 => {
                if let Some(d) = dest {
                    if !self.try_emit_fpxor_sign_mask(d, args, 8, true, "vxorpd") {
                        self.emit_avx_binary_256(d, args, "vxorpd", true);
                    }
                }
            }
            IntrinsicOp::VecXorF32x4 => {
                if let Some(d) = dest {
                    if !self.try_emit_fpxor_sign_mask(d, args, 4, false, "vxorps") {
                        self.emit_sse_binary_128(d, args, "xorps");
                    }
                }
            }
            IntrinsicOp::VecXorF64x2 => {
                if let Some(d) = dest {
                    if !self.try_emit_fpxor_sign_mask(d, args, 8, false, "vxorpd") {
                        self.emit_sse_binary_128(d, args, "xorpd");
                    }
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
            // Packed min/max. `commutative = false` is a correctness
            // requirement, not a tuning choice: MINPS/MAXPS return the SECOND
            // source on unordered or both-zero lanes, so `min(a, b)` and
            // `min(b, a)` differ for NaN and ±0 exactly like the C ternaries
            // `a < b ? a : b` vs `b < a ? b : a` they implement.  The binary
            // helper's non-commutative path emits `op args[1], args[0], dst`
            // (VEX) / `op args[1], args[0]` (SSE), i.e. dst = op(args[0],
            // args[1]) with args[1] as the returned-on-unordered source.
            IntrinsicOp::VecMinF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vminps", false);
                }
            }
            IntrinsicOp::VecMaxF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vmaxps", false);
                }
            }
            IntrinsicOp::VecMinF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vminpd", false);
                }
            }
            IntrinsicOp::VecMaxF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vmaxpd", false);
                }
            }
            IntrinsicOp::VecMinF32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "minps");
                }
            }
            IntrinsicOp::VecMaxF32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "maxps");
                }
            }
            IntrinsicOp::VecMinF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "minpd");
                }
            }
            IntrinsicOp::VecMaxF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "maxpd");
                }
            }
            IntrinsicOp::VecCmpF32x8 | IntrinsicOp::VecCmpF64x4 => {
                if let Some(d) = dest {
                    let inst = if matches!(op, IntrinsicOp::VecCmpF32x8) {
                        "vcmpps"
                    } else {
                        "vcmppd"
                    };
                    self.emit_avx_cmp_256(d, args, inst);
                }
            }
            IntrinsicOp::VecCmpF32x4 | IntrinsicOp::VecCmpF64x2 => {
                if let Some(d) = dest {
                    let inst = if matches!(op, IntrinsicOp::VecCmpF32x4) {
                        "cmpps"
                    } else {
                        "cmppd"
                    };
                    self.emit_sse_cmp_128(d, args, inst);
                }
            }
            IntrinsicOp::VecBlendvF32x8 | IntrinsicOp::VecBlendvF64x4 => {
                if let Some(d) = dest {
                    let inst = if matches!(op, IntrinsicOp::VecBlendvF32x8) {
                        "vblendvps"
                    } else {
                        "vblendvpd"
                    };
                    self.emit_avx_blendv_256(d, args, inst);
                }
            }
            IntrinsicOp::VecBlendvF32x4 | IntrinsicOp::VecBlendvF64x2 => {
                if let Some(d) = dest {
                    let domain = if matches!(op, IntrinsicOp::VecBlendvF32x4) {
                        BlendvDomain::Ps
                    } else {
                        BlendvDomain::Pd
                    };
                    self.emit_sse_blendv_128(d, args, domain);
                }
            }
            IntrinsicOp::VecBlendvI32x8 => {
                if let Some(d) = dest {
                    // vblendvps is a bitwise dword-lane select keyed on the
                    // mask lane's sign bit — payload bits are copied
                    // verbatim, so the FP mnemonic is exact on I32/U32.
                    self.emit_avx_blendv_256(d, args, "vblendvps");
                }
            }
            IntrinsicOp::VecBlendvI8x32 => {
                if let Some(d) = dest {
                    // vpblendvb has the same operand semantics as
                    // vblendvps at byte granularity: each mask byte's sign
                    // bit selects the lane's source, payload copied
                    // verbatim — exact on I8/U8 payloads.
                    self.emit_avx_blendv_256(d, args, "vpblendvb");
                }
            }
            IntrinsicOp::VecBlendvI32x4 => {
                if let Some(d) = dest {
                    // Dword lanes: the vblendvps sign-bit select is exact
                    // at the lane granularity under SSE4.1+AVX2.
                    if self.isa.sse41 && self.avx2_enabled {
                        self.emit_blendv_128_vex(d, args, "vblendvps");
                    } else {
                        self.emit_sse_blendv_128_int(d, args);
                    }
                }
            }
            IntrinsicOp::VecCmpI32x8 | IntrinsicOp::VecCmpI32x4 => {
                if let Some(d) = dest {
                    let avx2 = matches!(op, IntrinsicOp::VecCmpI32x8);
                    self.emit_int_cmp(d, args, avx2, IntCmpLane::Dword);
                }
            }
            // ---- Byte-lane (I8/U8) map ops -------------------------------
            //
            // 32 elements per YMM instead of the 8 an `int`-promoted tree
            // gets.  Each of these reuses the SAME defer-aware helper as its
            // dword twin, so the all-homed three-operand VEX fast paths, the
            // VLFOLD memory folding and the deferred-store discipline apply
            // unchanged.
            IntrinsicOp::VecCmpI8x32 | IntrinsicOp::VecCmpI8x16 => {
                if let Some(d) = dest {
                    let avx2 = matches!(op, IntrinsicOp::VecCmpI8x32);
                    self.emit_int_cmp(d, args, avx2, IntCmpLane::Byte);
                }
            }
            IntrinsicOp::VecAddI8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpaddb", true);
                }
            }
            IntrinsicOp::VecAddI8x16 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "paddb");
                }
            }
            // Subtract is NON-commutative: `dest = src1 - src2`, and only the
            // src2 slot may carry a folded memory operand.
            IntrinsicOp::VecSubI8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpsubb", false);
                }
            }
            IntrinsicOp::VecSubI8x16 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "psubb");
                }
            }
            // Unsigned byte min/max are SSE2 baseline instructions (unlike
            // the dword forms, which need SSE4.1), so the byte clamp idiom
            // lowers to two instructions on EVERY x86-64 target.
            IntrinsicOp::VecMinU8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpminub", true);
                }
            }
            IntrinsicOp::VecMinU8x16 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pminub");
                }
            }
            IntrinsicOp::VecMaxU8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaxub", true);
                }
            }
            IntrinsicOp::VecMaxU8x16 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pmaxub");
                }
            }
            // Signed byte min/max are SSE4.1 (`pminsb`/`pmaxsb`), so unlike
            // the unsigned pair they exist only on the AVX2 path; the 128-bit
            // baseline keeps the compare+blend form the demotion emits when
            // this op is unavailable.
            IntrinsicOp::VecMinI8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpminsb", true);
                }
            }
            IntrinsicOp::VecMaxI8x32 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaxsb", true);
                }
            }
            // ---- Word-lane (I16/U16) map ops -----------------------------
            IntrinsicOp::VecCmpI16x16 | IntrinsicOp::VecCmpI16x8 => {
                if let Some(d) = dest {
                    let avx2 = matches!(op, IntrinsicOp::VecCmpI16x16);
                    self.emit_int_cmp(d, args, avx2, IntCmpLane::Word);
                }
            }
            IntrinsicOp::VecAddI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpaddw", true);
                }
            }
            IntrinsicOp::VecAddI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "paddw");
                }
            }
            IntrinsicOp::VecSubI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpsubw", false);
                }
            }
            IntrinsicOp::VecSubI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "psubw");
                }
            }
            // Words DO have a packed multiply (bytes do not); `pmullw` keeps
            // the low half, which is exact for both signednesses.
            IntrinsicOp::VecMulI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmullw", true);
                }
            }
            IntrinsicOp::VecMulI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pmullw");
                }
            }
            // Signed word min/max are SSE2 BASELINE (unlike both the byte and
            // the dword signed forms), so both widths have them.
            IntrinsicOp::VecMinI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpminsw", true);
                }
            }
            IntrinsicOp::VecMinI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pminsw");
                }
            }
            IntrinsicOp::VecSminI32x4 => {
                if let Some(d) = dest {
                    self.emit_sminmax_i32x4(d, args, false);
                }
            }
            IntrinsicOp::VecSmaxI32x4 => {
                if let Some(d) = dest {
                    self.emit_sminmax_i32x4(d, args, true);
                }
            }
            IntrinsicOp::VecMaxI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaxsw", true);
                }
            }
            IntrinsicOp::VecMaxI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pmaxsw");
                }
            }
            // BB-SLP packed lane shifts by a uniform constant. The VEX
            // immediate forms are three-operand; the SSE2 forms are
            // destructive (the emitters stage through the destination
            // home or %xmm0). Per-family amount bounds live HERE, where
            // the lane width is known: 15 (word), 31 (dword), 63 (qword).
            IntrinsicOp::VecShlI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsllw", 15);
                }
            }
            IntrinsicOp::VecShlI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psllw", 15);
                }
            }
            IntrinsicOp::VecLShrI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsrlw", 15);
                }
            }
            IntrinsicOp::VecLShrI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psrlw", 15);
                }
            }
            IntrinsicOp::VecAShrI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsraw", 15);
                }
            }
            IntrinsicOp::VecAShrI16x8 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psraw", 15);
                }
            }
            IntrinsicOp::VecShlI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpslld", 31);
                }
            }
            IntrinsicOp::VecShlI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "pslld", 31);
                }
            }
            IntrinsicOp::VecLShrI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsrld", 31);
                }
            }
            IntrinsicOp::VecLShrI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psrld", 31);
                }
            }
            IntrinsicOp::VecAShrI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsrad", 31);
                }
            }
            IntrinsicOp::VecAShrI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psrad", 31);
                }
            }
            IntrinsicOp::VecShlI64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsllq", 63);
                }
            }
            IntrinsicOp::VecShlI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psllq", 63);
                }
            }
            IntrinsicOp::VecLShrI64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_shift_imm_256(d, args, "vpsrlq", 63);
                }
            }
            IntrinsicOp::VecLShrI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_shift_imm_128(d, args, "psrlq", 63);
                }
            }
            // Unsigned word min/max are SSE4.1, hence AVX2-only here.
            IntrinsicOp::VecMinU16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpminuw", true);
                }
            }
            IntrinsicOp::VecMaxU16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaxuw", true);
                }
            }
            // A word compare sets all 16 bits of its lane, so both bytes of
            // every lane carry the same sign bit and the per-BYTE blend is
            // exact at word granularity.
            IntrinsicOp::VecBlendvI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_blendv_256(d, args, "vpblendvb");
                }
            }
            IntrinsicOp::VecBlendvI16x8 => {
                if let Some(d) = dest {
                    // Word lanes: `vpblendvb` consults the sign bit of
                    // EVERY BYTE, and a word-compare mask sets all 16 bits
                    // of its lane — both bytes carry the same value, so
                    // the per-byte select is word-atomic and exact. The
                    // DWORD `vblendvps` would read only the HIGH word's
                    // mask into bits 31 of each dword — wrong whenever a
                    // dword's two word lanes disagree.
                    if self.isa.sse41 && self.avx2_enabled {
                        self.emit_blendv_128_vex(d, args, "vpblendvb");
                    } else {
                        self.emit_sse_blendv_128_int(d, args);
                    }
                }
            }
            IntrinsicOp::VecBroadcastI16x16 => {
                // Zero splat: one vpxor instead of the staged chain (the
                // families with no VecZero intrinsic honor the same
                // is_zero contract emitter-locally).
                if !self.try_zero_splat(args, dest, true)
                    && !self.try_all_ones_splat(args, dest, true)
                {
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    vpbroadcastw %xmm0, %ymm0");
                    self.state.dirty_upper_ymm = true;
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.avx_store_dest(d);
                        self.flush_pending_vec_store_impl();
                    }
                }
            }
            IntrinsicOp::VecBroadcastI16x8 => {
                // Zero splat then all-ones: one instruction each instead of
                // the staged movd/punpcklwd/pshufd chain.
                if !self.try_zero_splat(args, dest, false)
                    && !self.try_all_ones_splat(args, dest, false)
                {
                    self.state.invalidate_vec_peephole();
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            // Byte splat of a RUNTIME value.  `vpbroadcastb` takes its source
            // from an XMM lane, so a GPR operand goes through `vmovd` first.
            IntrinsicOp::VecBroadcastI8x32 => {
                // Zero splat then all-ones: one instruction each instead of
                // the staged movd/vpbroadcastb chain.
                if !self.try_zero_splat(args, dest, true)
                    && !self.try_all_ones_splat(args, dest, true)
                {
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    vpbroadcastb %xmm0, %ymm0");
                    self.state.dirty_upper_ymm = true;
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.avx_store_dest(d);
                        self.flush_pending_vec_store_impl();
                    }
                }
            }
            // SSE2 baseline byte splat: `pshufb` is SSSE3, so build the
            // broadcast from unpacks.  b -> {b,b} -> {b,b,b,b} -> all lanes.
            IntrinsicOp::VecBroadcastI8x16 => {
                // Zero splat then all-ones: one instruction each instead of
                // the movd/punpcklbw/punpcklwd/pshufd chain.
                if !self.try_zero_splat(args, dest, false)
                    && !self.try_all_ones_splat(args, dest, false)
                {
                    self.state.invalidate_vec_peephole();
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklbw %xmm0, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            // `vpblendvb` consults the sign bit of EVERY BYTE; the dword
            // blend (`vblendvps`) would smear one mask bit across four bytes
            // and is therefore never a legal substitute here.
            // The 128-bit bitwise select `(t & m) | (f & ~m)` is exact for a
            // byte mask as well (it is a per-BIT select), so the existing
            // SSE2 lowering serves the byte width with no SSE4.1 dependency.
            IntrinsicOp::VecBlendvI8x16 => {
                if let Some(d) = dest {
                    // Byte lanes: `vpblendvb` (per-BYTE sign select) is
                    // the exact one-instruction form; the dword blend
                    // would smear one mask bit across four bytes.
                    if self.isa.sse41 && self.avx2_enabled {
                        self.emit_blendv_128_vex(d, args, "vpblendvb");
                    } else {
                        self.emit_sse_blendv_128_int(d, args);
                    }
                }
            }
            // ---- SSE2 byte-lane twins (baseline path) ----------------------
            // Same defer-aware helpers as the dword x4 forms.
            // Subtract is NON-commutative: `dest = src1 - src2`, and only
            // the src2 slot may carry a folded memory operand.
            // Unsigned byte min/max are SSE2 BASELINE instructions (unlike
            // the dword forms, which need SSE4.1), so the byte clamp idiom
            // lowers to two instructions on every x86-64 target.
            // `vpblendvb` consults the sign bit of EVERY BYTE; the dword
            // blend would smear one mask bit across four bytes.  The
            // 128-bit form is the lane-agnostic bitwise select
            // `(t & m) | (f & ~m)` — exact for a byte mask at SSE2.
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
            IntrinsicOp::VecMaddF64x4Signed(np, na) => {
                if let Some(d) = dest {
                    // Builtin-semantics packed FMA with the sign algebra
                    // (MapExpr::Fma from __builtin_fma loops): same
                    // [input, scale, bias] operand movement as the plain
                    // affine madd, family selected by (np, na). The
                    // 132-family mnemonic is what emit_avx_map_fma's
                    // memory-fold re-encoding rewrites (213/231), so the
                    // signed families fold loads exactly like the plain one.
                    let mn = match (np, na) {
                        (false, false) => "vfmadd132pd",
                        (false, true) => "vfmsub132pd",
                        (true, false) => "vfnmadd132pd",
                        (true, true) => "vfnmsub132pd",
                    };
                    self.emit_avx_map_fma(d, args, mn);
                }
            }
            IntrinsicOp::VecMaddF32x8Signed(np, na) => {
                if let Some(d) = dest {
                    let mn = match (np, na) {
                        (false, false) => "vfmadd132ps",
                        (false, true) => "vfmsub132ps",
                        (true, false) => "vfnmadd132ps",
                        (true, true) => "vfnmsub132ps",
                    };
                    self.emit_avx_map_fma(d, args, mn);
                }
            }
            IntrinsicOp::VecFmaF64x2
            | IntrinsicOp::VecFnmaF64x2
            | IntrinsicOp::VecFmaF32x4
            | IntrinsicOp::VecFnmaF32x4 => {
                if let Some(d) = dest {
                    // args = [a, b, acc] → acc ± a·b, 128-bit lanes.
                    let (mn, ps) = match op {
                        IntrinsicOp::VecFmaF64x2 => ("vfmadd", "pd"),
                        IntrinsicOp::VecFnmaF64x2 => ("vfnmadd", "pd"),
                        IntrinsicOp::VecFmaF32x4 => ("vfmadd", "ps"),
                        _ => ("vfnmadd", "ps"),
                    };
                    self.emit_vec_fma_128(d, args, mn, ps);
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
            IntrinsicOp::VecRotlI32x4 => {
                if let Some(d) = dest {
                    // A non-constant amount silently became rotl(0) here
                    // (a whole-vector identity) in release builds, where
                    // the old `debug_assert!` never fired. The ARX pass
                    // and the SLP pack builder only ever emit constants,
                    // so a non-const operand is a compiler bug — fail
                    // loudly instead of miscompiling.
                    let n = match &args[1] {
                        Operand::Const(c) => c.to_i64().unwrap_or(0) as i32,
                        _ => panic!("VecRotlI32x4: rotate amount must be a constant"),
                    };
                    assert!((1..=31).contains(&n), "VecRotlI32x4 bad amount {n}");
                    self.state.invalidate_vec_peephole();
                    if self.avx512vl_enabled {
                        // AVX-512VL: `vprold` rotates every lane in ONE µop —
                        // the AVX2 form spends three (vpslld/vpsrld/vpor) and
                        // a scratch register.  ChaCha20's double round holds
                        // eight rotates; measured −18.7% end-to-end vs the
                        // AVX2 form with -march=native on AVX512VL silicon
                        // (240.7ms vs 296.1ms median-of-9, taskset;
                        // PERF-PROVENANCE-S49 §1b).  The xmm form is #UD
                        // without VL (EVEX.L'L=00), hence the dedicated gate.
                        let src = self.vex128_source(&args[0], "xmm0");
                        let dst_home = self.dest_xmm_home_name(d);
                        let dst = match dst_home {
                            Some(name) => format!("%{}", name),
                            None => "%xmm0".to_string(),
                        };
                        self.state
                            .emit_fmt(format_args!("    vprold ${}, {}, {}", n, src, dst));
                        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                        self.sse_commit_dest_direct(d, dst_static);
                        if dst_home.is_none() {
                            let deferred = self.state.vector_defer_values.contains(&d.0);
                            use crate::backend::state::SlotAddr;
                            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                self.state.resolve_slot_addr(d.0)
                            {
                                if !deferred {
                                    self.state.emit_fmt(format_args!(
                                        "    movdqu %xmm0, {}",
                                        self.slot_ref(slot.0)
                                    ));
                                } else {
                                    self.state.pending_vec_store = Some((d.0, "xmm0", false));
                                }
                            }
                        }
                    } else if self.avx2_enabled {
                        let src = self.vex128_source(&args[0], "xmm0");
                        let dst_home = self.dest_xmm_home_name(d);
                        let dst = match dst_home {
                            Some(name) => format!("%{}", name),
                            None => "%xmm0".to_string(),
                        };
                        self.state
                            .emit_fmt(format_args!("    vpsrld ${}, {}, %xmm1", 32 - n, src));
                        self.state
                            .emit_fmt(format_args!("    vpslld ${}, {}, {}", n, src, dst));
                        self.state
                            .emit_fmt(format_args!("    vpor %xmm1, {}, {}", dst, dst));
                        // Commit BEFORE creating the dest's own deferred store: the
                        // commit clears a pending naming the committed value (a SOURCE
                        // pending consumed by this write); the dest's own pending must
                        // survive it (the hsum stale-slot miscompile class).
                        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                        self.sse_commit_dest_direct(d, dst_static);
                        if dst_home.is_none() {
                            let deferred = self.state.vector_defer_values.contains(&d.0);
                            use crate::backend::state::SlotAddr;
                            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                self.state.resolve_slot_addr(d.0)
                            {
                                if !deferred {
                                    self.state.emit_fmt(format_args!(
                                        "    movdqu %xmm0, {}",
                                        self.slot_ref(slot.0)
                                    ));
                                } else {
                                    self.state.pending_vec_store = Some((d.0, "xmm0", false));
                                }
                            }
                        }
                    } else {
                        // Legacy SSE2 discipline: the dest-homed fast path
                        // in the shared helper (coalesced in-place forms,
                        // pending-store-aware staging).
                        self.emit_int_rotl_i32x4(d, args);
                    }
                }
            }
            IntrinsicOp::VecShufdI32x4 => {
                if let Some(d) = dest {
                    // Same hardening as VecRotlI32x4: a non-constant
                    // immediate silently became 0xE4 (a lane permutation)
                    // in release builds — a compiler bug must fail loudly.
                    let imm = match &args[1] {
                        Operand::Const(c) => c.to_i64().unwrap_or(0xE4) as i32,
                        _ => panic!("VecShufdI32x4: shuffle immediate must be a constant"),
                    };
                    assert!(
                        (0..=255).contains(&imm),
                        "VecShufdI32x4 imm8 out of range: {imm}"
                    );
                    self.state.invalidate_vec_peephole();
                    if self.avx2_enabled {
                        let src = self.vex128_source(&args[0], "xmm0");
                        let dst_home = self.dest_xmm_home_name(d);
                        let dst = match dst_home {
                            Some(name) => format!("%{}", name),
                            None => "%xmm0".to_string(),
                        };
                        self.state
                            .emit_fmt(format_args!("    vpshufd ${}, {}, {}", imm, src, dst));
                        // Commit BEFORE creating the dest's own deferred store: the
                        // commit clears a pending naming the committed value (a SOURCE
                        // pending consumed by this write); the dest's own pending must
                        // survive it (the hsum stale-slot miscompile class).
                        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                        self.sse_commit_dest_direct(d, dst_static);
                        if dst_home.is_none() {
                            let deferred = self.state.vector_defer_values.contains(&d.0);
                            use crate::backend::state::SlotAddr;
                            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                self.state.resolve_slot_addr(d.0)
                            {
                                if !deferred {
                                    self.state.emit_fmt(format_args!(
                                        "    movdqu %xmm0, {}",
                                        self.slot_ref(slot.0)
                                    ));
                                } else {
                                    self.state.pending_vec_store = Some((d.0, "xmm0", false));
                                }
                            }
                        }
                    } else {
                        // Legacy SSE2 discipline: the dest-homed fast path
                        // in the shared helper (coalesced in-place forms,
                        // pending-store-aware staging).
                        self.emit_int_shufd_i32x4(d, args);
                    }
                }
            }
            IntrinsicOp::VecShufbI32x4 => {
                if let Some(d) = dest {
                    self.state.invalidate_vec_peephole();
                    if self.avx2_enabled {
                        let mask = self.vex128_source(&args[1], "xmm1");
                        let src = self.vex128_source(&args[0], "xmm0");
                        let dst_home = self.dest_xmm_home_name(d);
                        let dst = match dst_home {
                            Some(name) => format!("%{}", name),
                            None => "%xmm0".to_string(),
                        };
                        self.state
                            .emit_fmt(format_args!("    vpshufb {}, {}, {}", mask, src, dst));
                        // Commit BEFORE creating the dest's own deferred store: the
                        // commit clears a pending naming the committed value (a SOURCE
                        // pending consumed by this write); the dest's own pending must
                        // survive it (the hsum stale-slot miscompile class).
                        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
                        self.sse_commit_dest_direct(d, dst_static);
                        if dst_home.is_none() {
                            let deferred = self.state.vector_defer_values.contains(&d.0);
                            use crate::backend::state::SlotAddr;
                            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                self.state.resolve_slot_addr(d.0)
                            {
                                if !deferred {
                                    self.state.emit_fmt(format_args!(
                                        "    movdqu %xmm0, {}",
                                        self.slot_ref(slot.0)
                                    ));
                                } else {
                                    self.state.pending_vec_store = Some((d.0, "xmm0", false));
                                }
                            }
                        }
                    } else {
                        // Legacy SSE2 discipline: the dest-homed fast path
                        // in the shared helper (coalesced in-place forms,
                        // pending-store-aware staging).
                        self.emit_int_shufb_i32x4(d, args);
                    }
                }
            }
            IntrinsicOp::VecPackI32x4 => {
                if let Some(d) = dest {
                    self.emit_int_pack_i32x4(d, args);
                }
            }
            IntrinsicOp::VecExtractLaneI32x4 => {
                self.emit_int_extract_i32x4(dest.as_ref(), args);
            }
            IntrinsicOp::VecMulI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pmulld");
                }
            }
            // Integer map lane ops. Sub is `src1 - src2` and NON-commutative:
            // the VLFOLD memory operand is only legal in the src2 slot, and
            // `emit_sse_binary_128` keeps args[0] as the destination-side
            // operand of the two-operand SSE form. And/Or/Xor are bit-exact
            // and commutative; `vpand`/`vpor`/`vpxor` and their SSE2 forms
            // are 1-cycle, 3/cycle on p015 — the cheapest 256-bit ops the
            // machine has.
            IntrinsicOp::VecSubI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpsubd", false);
                }
            }
            IntrinsicOp::VecSubI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "psubd");
                }
            }
            IntrinsicOp::VecSubI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "psubq");
                }
            }
            IntrinsicOp::VecAndI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpand", true);
                }
            }
            IntrinsicOp::VecAndI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pand");
                }
            }
            IntrinsicOp::VecOrI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpor", true);
                }
            }
            IntrinsicOp::VecOrI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "por");
                }
            }
            IntrinsicOp::VecXorI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpxor", true);
                }
            }
            IntrinsicOp::VecAndI8x32 | IntrinsicOp::VecAndI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpand", true);
                }
            }
            IntrinsicOp::VecOrI8x32 | IntrinsicOp::VecOrI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpor", true);
                }
            }
            IntrinsicOp::VecXorI8x32 | IntrinsicOp::VecXorI16x16 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpxor", true);
                }
            }
            IntrinsicOp::VecXorI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pxor");
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
            IntrinsicOp::VecMinI32x8 => {
                // Lane-wise signed min of two 8×I32 vectors (vpminsd). The
                // map vectorizer's integer min/max fold lowers exact integer
                // ternaries to it (clamp shapes: vpmaxsd + vpminsd).
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpminsd", true);
                }
            }
            IntrinsicOp::VecBroadcastI32x4 => {
                // All-ones splat: one self-compare instead of
                // movq/movd/pshufd.
                if !self.try_all_ones_splat(args, dest, false) {
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
            }
            IntrinsicOp::VecBroadcastI32x8 => {
                // All-ones splat: one vpcmpeqd instead of the staged chain.
                if !self.try_all_ones_splat(args, dest, true) {
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
            }
            IntrinsicOp::VecStoreI32x4 => {
                // Register-home source: store straight from the assigned XMM
                // register (mirrors the FP stores; see VecStoreI32x8).
                let src = self.vec_store_source_128(&args[0]);
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "movdqu", src);
                    return;
                }
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
            IntrinsicOp::VecStoreI32x8
            | IntrinsicOp::VecStoreI8x32
            | IntrinsicOp::VecStoreI16x16 => {
                // Register-home source: store straight from the assigned YMM
                // register (no slot round trip), exactly like the FP stores.
                // Without this, an RA-homed source (e.g. an integer min/max
                // result) stored stale %ymm0 contents — the deferred-scratch
                // discipline only keeps unhomed results in %ymm0.
                let src = self.vec_store_source_256(&args[0]);
                if src != "ymm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "vmovdqu", src);
                    return;
                }
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
                // AVX fast paths (the nbody mass/mag broadcasts): a
                // register-homed source broadcasts in place
                // (`vmovddup %xmmN, %xmmD`), a slot-homed one straight
                // from memory (`vmovddup SLOT, %xmmD`) — ONE instruction
                // where the SSE staging paid movsd+unpcklpd+movdqa (and
                // the staging copy `vmovsd %xmmN, %xmmN` was pure waste).
                if self.avx2_enabled {
                    if let Some(d) = dest {
                        let dst_home = self.dest_xmm_home_name(d);
                        let mut done = false;
                        if let Operand::Value(v) = &args[0] {
                            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                if is_xmm_reg(reg) {
                                    let name = phys_reg_name(reg);
                                    match dst_home {
                                        Some(h) if h == name => {
                                            // dest reuses the dying source's
                                            // home: broadcast in place.
                                            self.state.emit_fmt(format_args!(
                                                "    vmovddup %{}, %{}",
                                                name, name
                                            ));
                                        }
                                        Some(h) => {
                                            self.state.emit_fmt(format_args!(
                                                "    vmovddup %{}, %{}",
                                                name, h
                                            ));
                                        }
                                        None => {
                                            self.state.emit_fmt(format_args!(
                                                "    vmovddup %{}, %xmm0",
                                                name
                                            ));
                                        }
                                    }
                                    done = true;
                                }
                            }
                            if !done {
                                if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                                    self.state.resolve_slot_addr(v.0)
                                {
                                    match dst_home {
                                        Some(h) => {
                                            self.state.emit_fmt(format_args!(
                                                "    vmovddup {}, %{}",
                                                self.slot_ref(slot.0),
                                                h
                                            ));
                                        }
                                        None => {
                                            self.state.emit_fmt(format_args!(
                                                "    vmovddup {}, %xmm0",
                                                self.slot_ref(slot.0)
                                            ));
                                        }
                                    }
                                    done = true;
                                }
                            }
                        }
                        if !done {
                            self.emit_fp_operand_to_xmm(&args[0], IrType::F64, "xmm0");
                            self.state.emit("    unpcklpd %xmm0, %xmm0");
                            self.sse_store_dest(d, "xmm0");
                        } else if let Some(h) = dst_home {
                            self.sse_commit_dest_direct(d, h);
                        } else {
                            self.sse_store_dest(d, "xmm0");
                        }
                        self.state.vector_values.insert(d.0);
                    }
                } else {
                    self.emit_fp_operand_to_xmm(&args[0], IrType::F64, "xmm0");
                    self.state.emit("    unpcklpd %xmm0, %xmm0");
                    if let Some(d) = dest {
                        self.state.vector_values.insert(d.0);
                        self.sse_store_dest(d, "xmm0");
                    }
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
                // Register-home source: store straight from the assigned YMM
                // register (no slot round trip).  A pending deferred store to
                // this same value flowed through the register and is void.
                let src = self.vec_store_source_256(&args[0]);
                if src != "ymm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "vmovups", src);
                    return;
                }
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
                let src = self.vec_store_source_128(&args[0]);
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "movups", src);
                    return;
                }
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
                let src = self.vec_store_source_256(&args[0]);
                if src != "ymm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "vmovupd", src);
                    return;
                }
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
                let src = self.vec_store_source_128(&args[0]);
                if src != "xmm0" {
                    if let Operand::Value(v) = &args[0] {
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                            self.state.pending_vec_store = None;
                        }
                    }
                    self.state.invalidate_vec_peephole();
                    self.emit_vec_store_addr(args, dest_ptr, "movupd", src);
                    return;
                }
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
            IntrinsicOp::StrictRecipMulAddF64x4 => {
                // Strict computed-expression reduction, four elements at a
                // time.  This is intentionally NOT a horizontal reduction:
                // binary64 addition is observable under ordinary (non-fast)
                // C semantics, so the final four scalar additions must remain
                // acc+p0, then +p1, then +p2, then +p3.
                //
                // Args are [acc, denom0..denom3, source_base, byte_offset,
                // scratch].  The scalar denominator DAG is cloned by the pass
                // for each lane; packing only its already-computed I32 leaves
                // C's signed-div/mod semantics to the ordinary scalar IR.
                if args.len() != 8 {
                    panic!("StrictRecipMulAddF64x4 expects 8 args, got {}", args.len());
                }
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();

                // Pack d0..d3 into the low four i32 lanes, convert the full
                // XMM payload to four F64 lanes, and form 1.0 / denom.  All
                // scratch is confined to xmm0/ymm0 and xmm1/ymm1; the scalar
                // allocator deliberately reserves those families.
                self.operand_to_eax(&args[1]);
                self.state.emit("    movd %eax, %xmm0");
                for (lane, denom) in args[2..5].iter().enumerate() {
                    self.operand_to_eax(denom);
                    self.state
                        .emit_fmt(format_args!("    pinsrd ${}, %eax, %xmm0", lane + 1));
                }
                self.state.dirty_upper_ymm = true;
                self.state.emit("    vcvtdq2pd %xmm0, %ymm0");
                let one = self.vec_const_rip_operand(1.0f64.to_bits(), 8);
                self.state
                    .emit_fmt(format_args!("    vbroadcastsd {}, %ymm1", one));
                // AT&T VEX order is src2, src1, dst: ymm0 = ymm1 / ymm0.
                self.state.emit("    vdivpd %ymm0, %ymm1, %ymm0");

                // Form the contiguous source address before using rax for a
                // fallback scratch pointer; vec_mem_operand may materialize
                // non-register base/offset operands through rax/rcx.
                let source = self.vec_mem_operand(&args[5], &args[6], 0);
                // Preserve the source expression's multiplication order:
                // original scalar IR is reciprocal * load, hence src1=ymm0
                // and src2=the memory operand in this AT&T VEX form.
                self.state
                    .emit_fmt(format_args!("    vmulpd {}, %ymm0, %ymm0", source));

                // Fast path: the destination is a normal scalar F64 register
                // (xmm2..xmm15).  Keep the accumulator there, extract the
                // upper pair before rewriting the lower pair in ymm0, then
                // consume lanes in source order.  No vector value can be live
                // in xmm0/xmm1 across this intrinsic.
                let direct_dest = dest
                    .and_then(|d| self.reg_assignments.get(&d.0).copied())
                    .filter(|&r| is_xmm_reg(r))
                    .map(phys_reg_name)
                    .filter(|&name| name != "xmm0" && name != "xmm1");
                if let Some(name) = direct_dest {
                    self.state.reg_cache.invalidate_acc();
                    self.emit_fp_operand_to_xmm(&args[0], IrType::F64, name);
                    self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
                    // acc = acc + p0
                    self.state
                        .emit_fmt(format_args!("    vaddsd %xmm0, %{}, %{}", name, name));
                    // ymm0.low becomes {p1, p1}; its upper half is dead after
                    // the earlier extract, so the VEX.128 upper-zero rule is
                    // harmless.
                    self.state.emit("    vunpckhpd %xmm0, %xmm0, %xmm0");
                    // acc = acc + p1
                    self.state
                        .emit_fmt(format_args!("    vaddsd %xmm0, %{}, %{}", name, name));
                    // xmm1 initially holds {p2,p3}.
                    self.state
                        .emit_fmt(format_args!("    vaddsd %xmm1, %{}, %{}", name, name));
                    self.state.emit("    vunpckhpd %xmm1, %xmm1, %xmm1");
                    // acc = acc + p3
                    self.state
                        .emit_fmt(format_args!("    vaddsd %xmm1, %{}, %{}", name, name));
                } else {
                    // Register pressure must not turn into a correctness hole.
                    // The transform supplies a private aligned alloca so the
                    // packed product can be spilled once and read lane-by-lane
                    // without clobbering an allocator-owned XMM register.
                    self.operand_to_reg(&args[7], "rax");
                    self.state.emit("    vmovupd %ymm0, (%rax)");
                    self.state.reg_cache.invalidate_acc();
                    self.emit_fp_operand_to_xmm(&args[0], IrType::F64, "xmm0");
                    // Same VEX order as above: accumulator (src1) plus each
                    // lane memory operand (src2), strictly in lane order.
                    self.state.emit("    vaddsd (%rax), %xmm0, %xmm0");
                    self.state.emit("    vaddsd 8(%rax), %xmm0, %xmm0");
                    self.state.emit("    vaddsd 16(%rax), %xmm0, %xmm0");
                    self.state.emit("    vaddsd 24(%rax), %xmm0, %xmm0");
                    if let Some(d) = dest {
                        self.store_xmm_to(d, "xmm0", IrType::F64);
                    }
                }
            }
            IntrinsicOp::VecHorizontalAddF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - AVX2 4×F64 → F64.
                // The generic loader handles both a protected stack home and
                // a width-aware register assignment.
                self.avx_load_arg(&args[0]);
                // Follow-up #3: when the scalar result is register-homed
                // (the XMM scan gives the horizontal combine the same
                // register as the remainder loop's carry web), emit the
                // reduction steps directly into the destination register
                // with 3-operand VEX so no store/copy move is needed at
                // the loop entry.  The scratch for the cross-lane halves
                // is %xmm1.  SOUNDNESS (red-team audit): %xmm2 is IN the
                // F64/XMM scan pool (regalloc_helpers keeps xmm2-xmm7 unless
                // the function contains the listed xmm2-clobbering emitters),
                // so a different value can be register-homed in %xmm2 and
                // live across this intrinsic — clobbering it as scratch
                // would corrupt that value, and the dest==%xmm1 case is only
                // knowable AFTER allocation (the function-level clobbers_xmm2
                // gate cannot cover it).  dest==%xmm1 therefore routes through
                // the legacy sequence below (compute in %xmm0 with %xmm1
                // scratch, then one store_xmm_to move — %xmm1 is scratch-only
                // and never in any pool, so the legacy shape is always safe).
                // vaddpd writes all 128 bits of the dest, so the final
                // vaddsd sees no stale upper bits and the sequence has no
                // false dependency on anything older.
                let direct_dest = dest
                    .and_then(|d| self.reg_assignments.get(&d.0).copied())
                    .filter(|&r| is_xmm_reg(r))
                    .map(phys_reg_name)
                    .filter(|&name| name != "xmm1");
                if let Some(name) = direct_dest {
                    // phys_reg_name yields the bare mnemonic ("xmm9").
                    let scratch = "%xmm1";
                    self.state
                        .emit_fmt(format_args!("    vextractf128 $1, %ymm0, {}", scratch));
                    self.state
                        .emit_fmt(format_args!("    vaddpd {}, %xmm0, %{}", scratch, name));
                    // CRITICAL: unpack the SUMMED halves (the vaddpd
                    // result), not the pre-sum input — the high lane of
                    // the sum (b+d) is what the final scalar add needs.
                    self.state.emit_fmt(format_args!(
                        "    vunpckhpd %{}, %{}, {}",
                        name, name, scratch
                    ));
                    self.state
                        .emit_fmt(format_args!("    vaddsd {}, %{}, %{}", scratch, name, name));
                } else {
                    self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
                    self.state.emit("    vaddpd %xmm1, %xmm0, %xmm0");
                    self.state.emit("    vunpckhpd %xmm0, %xmm0, %xmm1");
                    self.state.emit("    vaddsd %xmm1, %xmm0, %xmm0");
                    if let Some(d) = dest {
                        self.store_xmm_to(d, "xmm0", IrType::F64);
                    }
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
                if let Some(d) = dest {
                    self.store_xmm_to(d, "xmm0", IrType::F64);
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
                            self.state.vec_claim_live_reg(d.0, name);
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
                // Follow-up #3: when the scalar result is register-homed
                // (XMM scan + FP copy web), emit the reduction directly
                // into the destination register with 3-operand VEX so no
                // store/copy move is needed at the remainder-loop entry.
                // Scratch is %xmm1.  SOUNDNESS (red-team audit): %xmm2 is IN
                // the F64/XMM scan pool (see the F64x4 arm above), so a value
                // homed in %xmm2 and live across this intrinsic would be
                // corrupted by using it as scratch; dest==%xmm1 is only
                // knowable after allocation, so it routes through the legacy
                // sequence below (always safe — %xmm1 is never in a pool).
                // vaddps writes all 128 bits of the dest, so the later steps
                // see no stale upper bits.
                let direct_dest = dest
                    .and_then(|d| self.reg_assignments.get(&d.0).copied())
                    .filter(|&r| is_xmm_reg(r))
                    .map(phys_reg_name)
                    .filter(|&name| name != "xmm1");
                if let Some(name) = direct_dest {
                    let scratch = "%xmm1";
                    self.state
                        .emit_fmt(format_args!("    vextractf128 $1, %ymm0, {}", scratch));
                    self.state
                        .emit_fmt(format_args!("    vaddps {}, %xmm0, %{}", scratch, name)); // [s0 s1 s2 s3]
                    self.state
                        .emit_fmt(format_args!("    vmovshdup %{}, {}", name, scratch)); // [s1 s1 s3 s3]
                    self.state
                        .emit_fmt(format_args!("    vaddps {}, %{}, %{}", scratch, name, name)); // [s0+s1, .., s2+s3, ..]
                    self.state.emit_fmt(format_args!(
                        "    vshufps $0xAA, %{}, %{}, {}",
                        name, name, scratch
                    )); // lanes {2,2,2,2}
                    self.state
                        .emit_fmt(format_args!("    vaddss {}, %{}, %{}", scratch, name, name)); // (s0+s1)+(s2+s3)
                } else {
                    self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
                    self.state.emit("    vaddps %xmm1, %xmm0, %xmm0"); // [s0 s1 s2 s3]
                    self.state.emit("    vmovshdup %xmm0, %xmm1"); // [s1 s1 s3 s3]
                    self.state.emit("    vaddps %xmm1, %xmm0, %xmm0"); // [s0+s1, .., s2+s3, ..]
                    self.state.emit("    vshufps $0xAA, %xmm0, %xmm0, %xmm1"); // lanes {2,2,2,2}
                    self.state.emit("    vaddss %xmm1, %xmm0, %xmm0"); // (s0+s1)+(s2+s3)
                    if let Some(d) = dest {
                        self.store_xmm_to(d, "xmm0", IrType::F32);
                    }
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
                if let Some(d) = dest {
                    self.store_xmm_to(d, "xmm0", IrType::F32);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                        self.state.vec_claim_live_reg(d.0, name);
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
                    // A silently skipped intrinsic leaves its destination
                    // uninitialised and the object file "successfully"
                    // compiled — the worst failure mode a compiler has.  The
                    // driver maps this panic to `ccc: internal error` and a
                    // non-zero exit (lib.rs compiler_main), so the build
                    // stops here instead of at run time.
                    panic!(
                        "unhandled intrinsic op {:?} (dest_ptr={}, args={}) — no x86-64 lowering \
                         for this operation under the active ISA; this is a compiler bug",
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
            .map(|pf| pf.val)
            .filter(|pv| {
                args.iter()
                    .any(|a| matches!(a, Operand::Value(v) if v.0 == *pv))
            });
        self.emit_avx_map_fma_inner(dest, args, mnemonic);
        if folded.is_some() {
            self.state.pending_vec_memfold = None;
        }
    }

    /// Re-encode a 132-family FMA mnemonic to its 213/231 sibling.
    ///
    /// The three FMA3 forms differ only in operand POSITION — the sign
    /// structure (±product ±addend) attaches to the algebraic terms, not
    /// the operand slots — so every family mnemonic has exact siblings
    /// that compute the same value with the memory operand in a different
    /// role (213: the addend; 231: a multiplicand). This is the complete,
    /// CLOSED mapping over the eight family mnemonics the map-FMA emitter
    /// accepts: an unknown string fails loudly instead of silently
    /// passing through. (The `str::replace("132", …)` this replaces was
    /// correct for all eight spellings but a silent no-op for any future
    /// mnemonic lacking a "132" infix — the wrong answer for a
    /// correctness-critical re-encoding.)
    fn fma_form_reencode(mnemonic: &str, to_231: bool) -> &'static str {
        match mnemonic {
            "vfmadd132pd" => {
                if to_231 {
                    "vfmadd231pd"
                } else {
                    "vfmadd213pd"
                }
            }
            "vfmadd132ps" => {
                if to_231 {
                    "vfmadd231ps"
                } else {
                    "vfmadd213ps"
                }
            }
            "vfmsub132pd" => {
                if to_231 {
                    "vfmsub231pd"
                } else {
                    "vfmsub213pd"
                }
            }
            "vfmsub132ps" => {
                if to_231 {
                    "vfmsub231ps"
                } else {
                    "vfmsub213ps"
                }
            }
            "vfnmadd132pd" => {
                if to_231 {
                    "vfnmadd231pd"
                } else {
                    "vfnmadd213pd"
                }
            }
            "vfnmadd132ps" => {
                if to_231 {
                    "vfnmadd231ps"
                } else {
                    "vfnmadd213ps"
                }
            }
            "vfnmsub132pd" => {
                if to_231 {
                    "vfnmsub231pd"
                } else {
                    "vfnmsub213pd"
                }
            }
            "vfnmsub132ps" => {
                if to_231 {
                    "vfnmsub231ps"
                } else {
                    "vfnmsub213ps"
                }
            }
            other => unreachable!("not a 132-family FMA mnemonic: {other}"),
        }
    }

    fn emit_avx_map_fma_inner(&mut self, dest: &Value, args: &[Operand], mnemonic: &str) {
        assert!(args.len() == 3, "{} expects input, scale, bias", mnemonic);
        // VLFOLD forms (ICX saxpy shape): one of the three operands is an
        // elided single-use load and the FMA takes its SOURCE memory
        // operand directly, exactly like the audited binary emitter
        // (`emit_avx_binary_256_inner`). The Intel form algebra, with the
        // sign structure attached to the algebraic TERMS (invariant under
        // the operand permutation, re-derived against the SDM for all
        // four families — the 132-family mnemonic re-encodes exactly):
        //   fold = addend      (213): `<fam213> MEM, %src2, %dst` where
        //                         dst holds a MULTIPLICAND (the streamed
        //                         input), src2 the other multiplicand
        //                         (typically the homed broadcast), MEM
        //                         the folded bias.
        //   fold = multiplicand (231): `<fam231> MEM, %src2, %dst` where
        //                         dst holds the ADDEND (the streamed
        //                         bias), src2 the other multiplicand,
        //                         MEM the folded input.
        // Operand resolution mirrors the binary arm: a register source is
        // the operand's RA home or the scratch register holding its
        // DEFERRED store (consumed in place — the pending store never
        // fires; 256-bit pendings only, a 128-bit-named register would
        // splice a mixed-width operand into a YMM instruction). The
        // destination takes the dst-operand's OWN home (free: the
        // single-use operand dies at this instruction — the classic
        // destructive-form register reuse), its deferred scratch, or a
        // load into a free scratch register.
        //
        // RESOLUTION IS MANDATORY (the binary arm's rule): when a fold
        // names one of this op's operands — guaranteed by the
        // emit_intrinsic_impl safety net — this arm either CONSUMES the
        // fold or falls through, and every ordinary path below re-issues
        // or materialises it. A pending fold is never left dangling past
        // this intrinsic, so no later fast path can read the phantom
        // register the allocator reserved for the elided value.
        if let (Some(pf), Operand::Value(m0), Operand::Value(m1), Operand::Value(bias)) = (
            self.state.pending_vec_memfold.clone(),
            &args[0],
            &args[1],
            &args[2],
        ) {
            let (pv, mem) = (pf.val, pf.mem.clone());
            // Memfold-aware home: the live fold's value has no
            // materialised register — the RA reserved one but the elided
            // load never wrote it. Reporting that register here is how a
            // consumer silently reads garbage (the v6 clamp class).
            let home = |this: &Self, v: &Value| -> Option<PhysReg> {
                if v.0 == pv {
                    return None;
                }
                this.reg_assignments
                    .get(&v.0)
                    .copied()
                    .filter(|r| is_xmm_reg(*r))
            };
            // The scratch register holding an operand's DEFERRED store,
            // if that single-use store is still pending.
            let held = |this: &Self, v: &Value| -> Option<&'static str> {
                this.state
                    .pending_vec_store
                    .filter(|(p, _, wide)| *p == v.0 && *wide)
                    .map(|(_, r, _)| r)
            };
            // Shared tail: emit `<fam> MEM, %src2, %dst` with the full
            // result bookkeeping. `dst` must already hold the dst-operand
            // (dying home / deferred scratch / staged load).
            //
            // DST-HOMING SOUNDNESS: an operand's register home may serve as
            // dst ONLY when the operand provably dies at this instruction —
            // `vector_dying_values` (single use, same block as the def)
            // plus a live-regs claim on that home in THIS block (the
            // claim map is reset at block boundaries, which is what
            // excludes the loop-invariant broadcast: one static use site,
            // read by every iteration, claimed only in the preheader).
            // Any other home is read-only (src2) or stays untouched.
            let dying_home = |this: &Self, v: &Value, home: PhysReg| -> bool {
                let name = phys_reg_name_256(home);
                this.state.vector_dying_values.contains(&v.0)
                    && this.state.vec_live_regs.get(&v.0).copied() == Some(name)
            };
            let mut emit_2xx = |this: &mut Self,
                                form: &'static str,
                                src2: String,
                                dst: &'static str,
                                consumed_pending: bool| {
                if consumed_pending {
                    // The held operand's store never fires: its only use
                    // is this instruction (the VDEFER single-use window).
                    this.state.pending_vec_store = None;
                }
                this.state.dirty_upper_ymm = true;
                this.state
                    .emit_fmt(format_args!("    {} {}, {}, %{}", form, mem, src2, dst));
                this.state.pending_vec_memfold = None;
                // The result now owns dst's bank: every OTHER value's
                // claim on it (the dying dst-operand's, in particular)
                // just ended.
                this.state.vec_evict_bank_except(dst, dest.0);
                this.state.vec_last_store_reg = false;
                this.avx_store_dest_from(dest, dst);
            };
            // 213: the fold is the addend. Both multiplicands must be
            // free of the fold (an aliased multiplicand would have to be
            // the memory operand AND a register source at once; the
            // matcher's alias-free gate makes this unreachable, the check
            // stays defensive).
            if bias.0 == pv && m0.0 != pv && m1.0 != pv {
                // Two orders (the multiplicands commute). PREFERENCE: an
                // order whose dst-operand dies in its home (zero staging,
                // the GCC/ICX saxpy shape) beats one that merely resolves
                // src2 and stages the dst-operand through a scratch — so
                // every order is checked for a dying-home dst BEFORE any
                // staging fallback is taken.
                let mut staged: Option<(String, bool, &Operand)> = None;
                for (d_op, s_op, d_arg) in [(m0, m1, &args[0]), (m1, m0, &args[1])] {
                    let src2 = home(self, s_op)
                        .map(|r| format!("%{}", phys_reg_name_256(r)))
                        .or_else(|| held(self, s_op).map(|h| format!("%{h}")));
                    let Some(src2) = src2 else {
                        continue;
                    };
                    let consumed = held(self, s_op).is_some();
                    if let Some(dr) = home(self, d_op).filter(|&r| dying_home(self, d_op, r)) {
                        let dst = phys_reg_name_256(dr);
                        emit_2xx(
                            self,
                            Self::fma_form_reencode(mnemonic, false),
                            src2,
                            dst,
                            consumed,
                        );
                        return;
                    }
                    if let Some(h) = held(self, d_op) {
                        emit_2xx(
                            self,
                            Self::fma_form_reencode(mnemonic, false),
                            src2,
                            h,
                            consumed,
                        );
                        return;
                    }
                    staged = staged.or(Some((src2, consumed, d_arg)));
                }
                if let Some((src2, consumed, d_arg)) = staged {
                    // Stage the dst-operand into the scratch the src2 is
                    // NOT using (avx_load_arg_to flushes any unrelated
                    // pending store itself and evicts stale bank claims).
                    let scratch: &'static str = if src2 == "%ymm0" { "ymm1" } else { "ymm0" };
                    self.avx_load_arg_to(d_arg, scratch);
                    emit_2xx(
                        self,
                        Self::fma_form_reencode(mnemonic, false),
                        src2,
                        scratch,
                        consumed,
                    );
                    return;
                }
            }
            // 231: the fold is a multiplicand. The addend streams in dst;
            // src2 is the OTHER multiplicand (which must not itself be
            // the fold — see above).
            if (m0.0 == pv || m1.0 == pv) && bias.0 != pv {
                let other = if m0.0 == pv { m1 } else { m0 };
                let src2 = home(self, other)
                    .map(|r| format!("%{}", phys_reg_name_256(r)))
                    .or_else(|| held(self, other).map(|h| format!("%{h}")));
                if let Some(src2) = src2 {
                    let consumed = held(self, other).is_some();
                    if let Some(dr) = home(self, bias).filter(|&r| dying_home(self, bias, r)) {
                        let dst = phys_reg_name_256(dr);
                        emit_2xx(
                            self,
                            Self::fma_form_reencode(mnemonic, true),
                            src2,
                            dst,
                            consumed,
                        );
                        return;
                    }
                    if let Some(h) = held(self, bias) {
                        emit_2xx(
                            self,
                            Self::fma_form_reencode(mnemonic, true),
                            src2,
                            h,
                            consumed,
                        );
                        return;
                    }
                    let scratch: &'static str = if src2 == "%ymm0" { "ymm1" } else { "ymm0" };
                    self.avx_load_arg_to(&args[2], scratch);
                    emit_2xx(
                        self,
                        Self::fma_form_reencode(mnemonic, true),
                        src2,
                        scratch,
                        consumed,
                    );
                    return;
                }
            }
            // No 2XX form applied (the fold aliases operands across
            // roles, or no register source resolved): fall through to the
            // hardened ordinary paths, which re-issue or materialise the
            // fold — the RESOLUTION IS MANDATORY rule above.
        }
        if let (Operand::Value(input), Operand::Value(scale), Operand::Value(bias)) =
            (&args[0], &args[1], &args[2])
        {
            let input_held = self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(input.0)
                && self.state.vec_last_store_reg_name == Some("ymm0");
            // Memfold-aware sources (see the VLFOLD arm's `home`): an
            // elided scale/bias has no materialised register, and the
            // fast path must not read the phantom home. An elided input
            // is excluded by `input_held` itself (an elided load never
            // stores, so no last-store entry can name it).
            let live_fold = |this: &Self, v: &Value| -> bool {
                this.state
                    .pending_vec_memfold
                    .as_ref()
                    .is_some_and(|pf| pf.val == v.0)
            };
            let scale_reg = self
                .reg_assignments
                .get(&scale.0)
                .copied()
                .filter(|r| is_xmm_reg(*r))
                .filter(|_| !live_fold(self, scale));
            let bias_reg = self
                .reg_assignments
                .get(&bias.0)
                .copied()
                .filter(|r| is_xmm_reg(*r))
                .filter(|_| !live_fold(self, bias));
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
            // Memfold-aware (LOAD-BEARING, mirrors `vec_home_256`): a value
            // whose load was elided into `pending_vec_memfold` has no
            // materialised contents — the register the allocator reserved
            // for it was never written. Reporting that register as a
            // source is how a consumer silently reads garbage; returning
            // None routes the operand through `avx_load_arg_to` (which
            // re-issues the load) or the `memfold_operand` memory source
            // in the (None, None) arm below.
            if this.memfold_operand(arg).is_some() {
                return None;
            }
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
                // VLFOLD soundness: an elided load leaves its home slot NEVER
                // WRITTEN, so a pending fold on the scale must be consumed as
                // the real source memory operand here — never through the
                // value's slot (the pre-fix vfmadd132pd read of an uninitialised
                // stack slot that miscompiled streamed `a*b+c` maps).
                let scale_mem = if let Some(mem) = self.memfold_operand(&args[1]) {
                    self.state.pending_vec_memfold = None;
                    mem
                } else {
                    self.value_ptr_mem_operand(sv.0).unwrap_or_else(|| {
                        unreachable!(
                            "vfmadd132 scale operand must be homed, tracked, or slot-homed"
                        )
                    })
                };
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm1, %ymm0", mnemonic, scale_mem));
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
        }
    }

    /// Register name for an XMM-homed vector operand (YMM view), or `None`
    /// for slot-homed / deferred / memfolded values.
    ///
    /// The memfold clause is LOAD-BEARING and must stay first.  A value whose
    /// load was elided into `pending_vec_memfold` has no materialised
    /// contents: the register the allocator reserved for it was never
    /// written.  Reporting that register as its "home" is how a consumer
    /// silently reads garbage, so an elided value is reported as un-homed and
    /// the caller routes it through `avx_load_arg_to` (which re-issues the
    /// load) or consumes it as the folded memory operand.
    fn vec_home_256(&self, arg: &Operand) -> Option<String> {
        if self.memfold_operand(arg).is_some() {
            return None;
        }
        let Operand::Value(v) = arg else {
            return None;
        };
        self.reg_assignments
            .get(&v.0)
            .copied()
            .filter(|r| is_xmm_reg(*r))
            .map(|r| format!("%{}", phys_reg_name_256(r)))
    }

    /// YMM home names for a whole operand list plus a destination, or `None`
    /// if ANY of them is not register-homed.
    ///
    /// This is the admission test for the "all-homed" three-operand VEX fast
    /// paths: when every source and the destination own an XMM family, the
    /// operation can be computed register-to-register with no `%ymm0/%ymm1`
    /// staging and no home-store copy at all.  Two properties make it sound:
    ///
    /// * `avx_store_dest` NEVER defers a register-homed destination -- it
    ///   emits the `vmovdqa %ymm0, %ymmN` immediately -- so a homed value's
    ///   home always holds its current contents.  There is no staleness
    ///   window to check.
    /// * A value whose load was elided into a pending VLFOLD memory operand
    ///   has no materialised register contents, so it is rejected here and
    ///   the caller's slow path materialises it.
    ///
    /// Returns the destination home last, so callers destructure
    /// `[src.., dest]`.
    fn all_vec_homes_256(&self, srcs: &[&Operand], dest: &Value) -> Option<Vec<String>> {
        let pending_fold = self.state.pending_vec_memfold.as_ref().map(|pf| pf.val);
        let mut out = Vec::with_capacity(srcs.len() + 1);
        for s in srcs {
            let Operand::Value(v) = s else {
                return None;
            };
            if pending_fold == Some(v.0) {
                return None;
            }
            out.push(self.vec_home_256(s)?);
        }
        if pending_fold == Some(dest.0) {
            return None;
        }
        let &reg = self.reg_assignments.get(&dest.0)?;
        if !is_xmm_reg(reg) {
            return None;
        }
        out.push(format!("%{}", phys_reg_name_256(reg)));
        Some(out)
    }

    /// Bookkeeping shared by every all-homed fast path: the destination now
    /// lives in `dest_name` and no scratch staging is outstanding.
    fn note_vec_dest_in_home(&mut self, dest: &Value, dest_name: &str) {
        let name: &'static str = phys_reg_name_256(
            self.reg_assignments
                .get(&dest.0)
                .copied()
                .expect("all-homed fast path requires a destination home"),
        );
        debug_assert_eq!(format!("%{}", name), dest_name);
        self.state.dirty_upper_ymm = true;
        self.state.vec_claim_live_reg(dest.0, name);
        self.state.vec_last_store_val = Some(dest.0);
        self.state.vec_last_store_reg = true;
        self.state.vec_last_store_reg_name = Some(name);
        self.state.reg_cache.invalidate_acc();
    }

    /// Same as `vec_home_256` with the XMM view (128-bit paths), including
    /// the memfold clause (see there).
    fn vec_home_128(&self, arg: &Operand) -> Option<String> {
        if self.memfold_operand(arg).is_some() {
            return None;
        }
        let Operand::Value(v) = arg else {
            return None;
        };
        self.reg_assignments
            .get(&v.0)
            .copied()
            .filter(|r| is_xmm_reg(*r))
            .map(|r| format!("%{}", phys_reg_name(r)))
    }

    /// Current machine location of a vector value's content as a register
    /// name (without '%'): the held deferred value's register, a
    /// block-local live register, or its RA home.  `None` when the value
    /// is only slot-homed.  Mirrors the resolution order of
    /// `sse_load_arg`'s peepholes so a fast path that reads the location
    /// directly emits exactly what the staged path would have loaded.
    fn vec_operand_reg(&self, v: &Value) -> Option<&'static str> {
        // VLFOLD defense (mirrors `vec_home_128`/`all_vec_homes_256`): a
        // value whose load was elided into `pending_vec_memfold` has NO
        // materialised contents — the register the allocator reserved was
        // never written. Reporting it here would hand the in-place paths a
        // garbage source; declining routes the caller through the memfold
        // consumers or the materialisation paths.
        if self
            .state
            .pending_vec_memfold
            .as_ref()
            .is_some_and(|pf| pf.val == v.0)
        {
            return None;
        }
        if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0) {
            return Some(self.state.sse_last_store_reg_name.unwrap_or("xmm0"));
        }
        if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
            // Only a 128-bit-named entry may feed a legacy/VEX.128
            // operand: the 256-bit emitters book YMM-named entries here,
            // and splicing one into an `addps %src, %dst` would encode an
            // illegal mixed-width instruction (caught loud by the
            // assembler's width check).  Decline and let the caller's
            // staged path load the value from its slot.
            if held.starts_with("xmm") {
                return Some(held);
            }
            return None;
        }
        self.reg_assignments
            .get(&v.0)
            .copied()
            .filter(|r| is_xmm_reg(*r))
            .map(|r| phys_reg_name(r))
    }

    /// Bookkeeping after an in-place op wrote `dest` into its home
    /// register `target`: the value is live in that register (block-local
    /// tracking + last-store peephole), exactly like the accumulator fast
    /// path.
    fn sse_mark_in_place(&mut self, dest: &Value, target: &'static str) {
        self.state.vec_claim_live_reg(dest.0, target);
        self.state.sse_last_store_val = Some(dest.0);
        self.state.sse_last_store_reg = true;
        self.state.sse_last_store_reg_name = Some(target);
    }

    /// Direct-read source for a VEX.128 3-operand form: the operand's RA
    /// home, its within-block live register, or a load into `scratch`.
    /// VEX forms are NON-destructive, so a homed source is read in place
    /// with zero staging copies — the entire point of the VEX.128 path.
    /// Returns the AT&T register operand ("%xmm5").
    fn vex128_source(&mut self, arg: &Operand, scratch: &'static str) -> String {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                    return format!("%{}", phys_reg_name(reg));
                }
            }
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                // Only a 128-bit-named entry may feed a VEX.128 source
                // operand: the 256-bit emitters book YMM-named entries
                // here, and formatting one would encode an illegal
                // mixed-width instruction.  Decline and fall through to
                // the staged load below.
                if held.starts_with("xmm") {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                    // The live-reg table stores the bare name ("xmm0");
                    // every caller formats the result as a raw AT&T
                    // operand and expects the '%' prefix, like the
                    // assignment and scratch paths below.
                    return format!("%{}", held);
                }
            }
        }
        self.sse_load_arg(arg, scratch);
        format!("%{}", scratch)
    }

    /// Commit a VEX.128 result that is ALREADY in `reg` (the 3-operand
    /// destination wrote it there): bookkeeping only, no copy. `reg` is
    /// the scratch/home register NAME ("xmm0", "xmm5").
    fn sse_commit_dest_direct(&mut self, d: &Value, reg: &'static str) {
        self.state.vec_claim_live_reg(d.0, reg);
        self.state.sse_last_store_val = Some(d.0);
        self.state.sse_last_store_reg = true;
        self.state.sse_last_store_reg_name = Some(reg);
        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(d.0) {
            self.state.pending_vec_store = None;
        }
    }

    /// The static register NAME ("xmm5") for a homed destination, if the
    /// RA assigned it an XMM register.
    fn dest_xmm_home_name(&self, d: &Value) -> Option<&'static str> {
        self.reg_assignments
            .get(&d.0)
            .copied()
            .filter(|r| is_xmm_reg(*r))
            .map(phys_reg_name)
    }

    /// Packed 128-bit FMA/FMS contraction (BB-SLP): `dest = args[2] ±
    /// args[0]·args[1]` under the FP-contract discipline. Two shapes:
    ///
    /// * VLFOLD fast path — the accumulator is an elided single-use
    ///   128-bit load (the SLP MemLoad pack's deferred tuple): the 132
    ///   form folds it as the memory operand, `v{f,n}madd132{ps,pd} %b,
    ///   MEM_acc, %a_dst` (dst preloaded with a) — ONE instruction, the
    ///   GCC nbody shape.
    /// * register path — the accumulator loads into the destination
    ///   register first (a no-op copy when the allocator coalesced dest
    ///   with the dying accumulator), then `v{f,n}madd231{ps,pd} %b, %a,
    ///   %dst`.
    ///
    /// FMA3-only: the caller gates on the ISA (the SLP pass refuses to
    /// build FMA packs without it), and the mnemonic is unconditional
    /// here exactly like the scalar vfmadd231 emitter.
    fn emit_vec_fma_128(&mut self, dest: &Value, args: &[Operand], mn: &str, ps: &str) {
        assert!(
            args.len() == 3,
            "emit_vec_fma_128: {} expects [a, b, acc]",
            mn
        );
        self.state.invalidate_vec_peephole();
        // A pending deferred store may name the accumulator or a
        // multiplicand; the FMA reads all three — flush first (the
        // pending discipline only survives one consumer).
        self.flush_pending_vec_store_impl();

        let pf = self.state.pending_vec_memfold.clone();
        let acc_fold: Option<String> = pf
            .as_ref()
            .filter(|pf| pf.width == 16 && matches!(&args[2], Operand::Value(v) if v.0 == pf.val))
            .map(|pf| pf.mem.clone());

        let dst_home = self.dest_xmm_home_name(dest);
        let (dst, dst_static): (String, &'static str) = match dst_home {
            Some(name) => (format!("%{}", name), name),
            None => ("%xmm0".to_string(), "xmm0"),
        };

        // ── Register-alias discipline ────────────────────────────────
        // Three operands (a, b, acc) are live at the FMA and any of them
        // may be the register the allocator REUSED for the dest (each
        // dies at the instruction). The Intel forms — canonical AT&T
        // spelling (src3/r/m, src2/vvvv, dst), semantics CPU-verified:
        //   132: D = D × src3 + src2 — r/m is the SECOND MULTIPLICAND,
        //        VVVV the ADDEND. D must hold one MULTIPLICAND.
        //   213: D = src2 × D + src3 — r/m is the ADDEND, VVVV the first
        //        multiplier. D must hold one MULTIPLICAND.
        //   231: D = src2 × src3 + D — r/m is the second multiplier, D
        //        the ADDEND (the accumulator).
        // The alias of dst with a/b/acc selects the form; the product
        // commutes, so the multiplier register is whichever multiplicand
        // dst does NOT hold. Multiple aliases are impossible (a, b, acc
        // are mutually distinct live values).
        //
        // Resolve the multiplicands first (homed: their register; unhomed:
        // staged into %xmm1/%xmm2). The resolution NAMES let the alias
        // checks below see live-reg claims too, not just RA homes.
        let b_src = self.vex128_source(&args[1], "xmm1");
        let a_src = self.vex128_source(&args[0], "xmm2");

        if let Some(mem) = acc_fold {
            // 213 with the accumulator folded as the memory operand — the
            // ONE form whose r/m slot is the ADDEND (132/231 put a
            // MULTIPLICAND in r/m). Spelling this as a 132 with the
            // memory in the middle operand slot computed a·acc + b — the
            // multiplier/addend roles inverted — and had never been
            // assembled before: the v6 safety-net tightening dead-pathed
            // this arm for its whole life, so no test ever executed it.
            // Canonical AT&T: `v{f,n}madd213{ps,pd} MEM_acc, %S2, %dst`
            // computes S2 × dst + MEM_acc = a·b + acc, dst holding a
            // multiplicand (loaded into dst when it holds neither).
            if a_src == dst && b_src == dst {
                // Impossible (distinct live values share no register);
                // fail loudly rather than guess.
                unreachable!("FMA dest aliases both multiplicands");
            }
            if a_src != dst && b_src != dst {
                // dst holds neither: load `a` into it (b is already
                // resolved to its own register/scratch).
                self.sse_load_arg(&args[0], dst_static);
                self.state.emit_fmt(format_args!(
                    "    {}213{} {}, {}, {}",
                    mn, ps, mem, b_src, dst
                ));
            } else {
                let s2 = if a_src == dst {
                    b_src.clone()
                } else {
                    a_src.clone()
                };
                self.state
                    .emit_fmt(format_args!("    {}213{} {}, {}, {}", mn, ps, mem, s2, dst));
            }
            self.state.pending_vec_memfold = None;
        } else {
            if let Some(pf) = pf {
                // A pending fold of a DIFFERENT value cannot survive the
                // register reads below (the staging may clobber the
                // never-materialised register).
                let _ = pf;
                self.materialize_pending_memfold();
            }
            if a_src == dst || b_src == dst {
                // dst holds a multiplicand: the 132 form with the
                // accumulator as the register src1. Resolving the acc
                // after the multiplicands cannot claim dst (it is
                // neither's scratch) — unless the allocator coalesced
                // dest with the ACC, which contradicts this branch
                // holding a multiplicand; the defensive move-aside
                // keeps that impossible case correct anyway.
                let s2 = if a_src == dst {
                    b_src.clone()
                } else {
                    a_src.clone()
                };
                let acc_src = self.vex128_source(&args[2], "xmm3");
                if acc_src == dst {
                    self.state
                        .emit_fmt(format_args!("    movdqa {}, %xmm3", dst));
                    self.state
                        .emit_fmt(format_args!("    {}132{} {}, %xmm3, {}", mn, ps, s2, dst));
                } else {
                    self.state.emit_fmt(format_args!(
                        "    {}132{} {}, {}, {}",
                        mn, ps, s2, acc_src, dst
                    ));
                }
            } else {
                // 231: dst holds the accumulator (copied in when the
                // allocator did not coalesce dest with the dying acc).
                let acc_src = self.vex128_source(&args[2], dst_static);
                if acc_src != dst {
                    self.state
                        .emit_fmt(format_args!("    movdqa {}, {}", acc_src, dst));
                }
                self.state.emit_fmt(format_args!(
                    "    {}231{} {}, {}, {}",
                    mn, ps, b_src, a_src, dst
                ));
            }
        }
        // Commit: homed dests claim their register (the FMA wrote it in
        // place); unhomed dests store %xmm0 to their slot — or defer the
        // store when the value is in the defer set (the VecRotlI32x4
        // commit discipline).
        self.sse_commit_dest_direct(dest, dst_static);
        if dst_home.is_none() {
            let deferred = self.state.vector_defer_values.contains(&dest.0);
            if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                self.state.resolve_slot_addr(dest.0)
            {
                if !deferred {
                    self.state
                        .emit_fmt(format_args!("    movdqu %xmm0, {}", self.slot_ref(slot.0)));
                } else {
                    self.state.pending_vec_store = Some((dest.0, "xmm0", false));
                }
                self.state.sse_last_store_slot = Some(slot.0);
            }
        }
    }

    /// VEX.128 three-operand binary: `inst src2, src1, dst` with
    /// dst = src1 OP src2 (AT&T operand order). Homed sources read their
    /// registers directly; unhomed stream through %xmm0/%xmm1; the result
    /// lands in the destination's RA home — or %xmm0 with the ordinary
    /// deferred-store chain when it has none. Replaces the legacy
    /// two-operand staging (movdqa+movdqa+op+movdqa) with ONE instruction
    /// for every homed shape.
    #[expect(dead_code)] // wired by the ARX emitters below when targeted
    pub(super) fn emit_vex_binary_128(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        assert!(
            args.len() >= 2,
            "emit_vex_binary_128: malformed intrinsic {} ({} args)",
            inst,
            args.len()
        );
        self.state.invalidate_vec_peephole();
        // src2 first: an unhomed src2 streams through %xmm1 so an unhomed
        // src1 can keep %xmm0 (both scratches distinct, VEX reads both
        // without destroying either).
        let src2 = self.vex128_source(&args[1], "xmm1");
        let src1 = self.vex128_source(&args[0], "xmm0");
        let dst_home = self.dest_xmm_home_name(dest_ptr);
        let dst = match dst_home {
            Some(name) => format!("%{}", name),
            None => "%xmm0".to_string(),
        };
        self.state
            .emit_fmt(format_args!("    {} {}, {}, {}", inst, src2, src1, dst));
        // Commit BEFORE creating the dest's own deferred store: the
        // commit clears a pending naming the committed value (a SOURCE
        // pending consumed by this write); the dest's own pending must
        // survive it (the hsum stale-slot miscompile class).
        let dst_static: &'static str = dst_home.unwrap_or("xmm0");
        self.sse_commit_dest_direct(dest_ptr, dst_static);
        if dst_home.is_none() {
            // Unhomed destination: mirror sse_store_dest's slot discipline,
            // minus the register copy the VEX form made unnecessary.
            let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
            use crate::backend::state::SlotAddr;
            if let Some(addr) = self.state.resolve_slot_addr(dest_ptr.0) {
                if let SlotAddr::Direct(slot) = addr {
                    if !deferred {
                        self.state
                            .emit_fmt(format_args!("    movdqu %xmm0, {}", self.slot_ref(slot.0)));
                    } else {
                        self.state.pending_vec_store = Some((dest_ptr.0, "xmm0", false));
                    }
                    self.state.sse_last_store_slot = Some(slot.0);
                } else if !deferred {
                    self.value_to_reg(dest_ptr, "rax");
                    self.state
                        .emit_fmt(format_args!("    movdqu %xmm0, (%rax)"));
                } else {
                    self.state.pending_vec_store = Some((dest_ptr.0, "xmm0", false));
                }
            } else if !deferred {
                self.value_to_reg(dest_ptr, "rax");
                self.state
                    .emit_fmt(format_args!("    movdqu %xmm0, (%rax)"));
            } else {
                self.state.pending_vec_store = Some((dest_ptr.0, "xmm0", false));
            }
        }
    }

    /// Memory source for a non-homed vector operand AFTER every pending
    /// deferred store has been flushed: a pending VLFOLD (elided load) is
    /// consumed as the real source operand (its home slot was never
    /// written), otherwise the value's slot.  Callers must have flushed;
    /// a value that is neither homed, memfolded nor slot-homed is an
    /// invariant violation, never a silent fall-through.
    fn vec_mem_source_after_flush(&mut self, arg: &Operand, what: &str) -> String {
        if let Some(mem) = self.memfold_operand(arg) {
            self.state.pending_vec_memfold = None;
            return mem;
        }
        let Operand::Value(v) = arg else {
            unreachable!("{}: vector operand must be a value", what);
        };
        self.value_ptr_mem_operand(v.0).unwrap_or_else(|| {
            unreachable!("{}: operand must be homed, folded, or slot-homed", what)
        })
    }

    /// Packed FP compare (AVX): `dest = args[0] PRED args[1]` as an all-ones /
    /// all-zeros lane mask, `args[2]` = CMPPS predicate immediate.
    /// AT&T `vcmpps $imm, src2, src1, dst` computes `src1 PRED src2`, so
    /// args[0] streams through %ymm0 (src1) and args[1] is the VEX source.
    /// Load order mirrors the binary emitters: args[1] is resolved FIRST so a
    /// deferred args[1] still sitting in %ymm0 is moved to %ymm1 before the
    /// args[0] load can overwrite it.
    ///
    /// FP-Neg fast path: `VecXorF*(vec, Const(-0.0))` with the sign mask
    /// folded as a .rodata MEMORY operand — ONE instruction for the whole
    /// pack, exactly the form GCC emits for packed `-x` lanes
    /// (`vxorps .LCVEC_0(%rip), %xmm0, %xmm0`). Returns false when the
    /// second operand is not the family's `-0.0` constant (the caller
    /// falls through to the generic two-value emitters).
    ///
    /// `lane_bytes` is 4 (F32) or 8 (F64); `wide` selects the 256-bit
    /// family; `vex_mnemonic` is the VEX form (`vxorps`/`vxorpd`). Under
    /// AVX the 3-operand VEX form takes the mask in the r/m slot; the
    /// legacy-SSE2 fallback (128-bit only) stages the source into the
    /// destination and uses the destructive 2-operand form
    /// `xorps mask(%rip), %dst` — the pool's `.p2align 4` satisfies the
    /// legacy 16-byte alignment requirement.
    ///
    /// Operand order: AT&T `vxorps MEM, src1, dst` — the mask takes the
    /// r/m (first textual) slot, the vector source keeps its home or
    /// streams through the scratch pair exactly like the memfold consumers
    /// (a pending VLFOLD on args[0] is materialised first: this emitter
    /// cannot fold a second memory operand into the same instruction).
    fn try_emit_fpxor_sign_mask(
        &mut self,
        dest: &Value,
        args: &[Operand],
        lane_bytes: usize,
        wide: bool,
        vex_mnemonic: &str,
    ) -> bool {
        // The exact `-0.0` constant of the family's lane type.
        let is_minus_zero = match args.get(1) {
            Some(Operand::Const(IrConst::F32(v))) => lane_bytes == 4 && v.to_bits() == 0x8000_0000,
            Some(Operand::Const(IrConst::F64(v))) => {
                lane_bytes == 8 && v.to_bits() == 0x8000_0000_0000_0000
            }
            _ => false,
        };
        if !is_minus_zero {
            return false;
        }
        // A pending VLFOLD names args[0] (the analysis admits Xor consumers
        // in either position of the COMMUTATIVE op): this emitter's single
        // memory slot is the sign mask, so the fold must be materialised —
        // never read the elided value's never-written reserved register.
        if let Some(pf) = self.state.pending_vec_memfold.clone() {
            if matches!(&args[0], Operand::Value(v) if v.0 == pf.val) {
                self.materialize_pending_memfold();
            }
        }
        let total = if wide { 32 } else { 16 };
        let mut mask = vec![0u8; total];
        for l in 0..total / lane_bytes {
            mask[l * lane_bytes + lane_bytes - 1] = 0x80;
        }
        let label = self.state.get_vec_const_label(&mask);
        let mem = format!("{}(%rip)", label);
        // Legacy SSE2 (128-bit only — the 256-bit families require AVX):
        // the destructive two-operand form stages the source into the
        // destination, then XORs the aligned mask memory in place.
        if !self.avx2_enabled {
            debug_assert!(!wide, "256-bit families require AVX");
            let legacy = &vex_mnemonic[1..]; // "xorps" / "xorpd"
            match self.dest_xmm_home_name(dest) {
                Some(name) => {
                    if let Some(h) = self.vec_home_128(&args[0]) {
                        if h != format!("%{}", name) {
                            self.state
                                .emit_fmt(format_args!("    movdqa {}, %{}", h, name));
                        }
                    } else {
                        self.sse_load_arg(&args[0], name);
                    }
                    self.state
                        .emit_fmt(format_args!("    {} {}, %{}", legacy, mem, name));
                    self.sse_mark_in_place(dest, name);
                }
                None => {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} {}, %xmm0", legacy, mem));
                    self.sse_store_dest(dest, "xmm0");
                }
            }
            return true;
        }
        if wide {
            let dest_home = self
                .reg_assignments
                .get(&dest.0)
                .copied()
                .filter(|r| is_xmm_reg(*r));
            let src1 = match self.vec_home_256(&args[0]) {
                Some(h) => h,
                None => {
                    self.avx_load_arg(&args[0]);
                    "%ymm0".to_string()
                }
            };
            self.state.dirty_upper_ymm = true;
            match dest_home {
                Some(dr) => {
                    let dst = phys_reg_name_256(dr);
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}, %{}",
                        vex_mnemonic, mem, src1, dst
                    ));
                    self.state.vec_claim_live_reg(dest.0, dst);
                    self.state.vec_last_store_val = Some(dest.0);
                    self.state.vec_last_store_reg = true;
                    self.state.vec_last_store_reg_name = Some(dst);
                }
                None => {
                    self.flush_pending_vec_store_impl();
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}, %ymm0",
                        vex_mnemonic, mem, src1
                    ));
                    self.state.vec_last_store_reg = false;
                    self.avx_store_dest(dest);
                }
            }
        } else {
            let dst_home = self.dest_xmm_home_name(dest);
            let src1 = match self.vec_home_128(&args[0]) {
                Some(h) => h,
                None => {
                    self.sse_load_arg(&args[0], "xmm0");
                    "%xmm0".to_string()
                }
            };
            let dst = match dst_home {
                Some(name) => format!("%{}", name),
                None => "%xmm0".to_string(),
            };
            self.state.emit_fmt(format_args!(
                "    {} {}, {}, {}",
                vex_mnemonic, mem, src1, dst
            ));
            // Commit BEFORE creating the dest's own deferred store: the
            // commit clears a pending naming the committed value (a SOURCE
            // pending consumed by this write); the dest's own pending must
            // survive it (the hsum stale-slot miscompile class).
            let dst_static: &'static str = dst_home.unwrap_or("xmm0");
            self.sse_commit_dest_direct(dest, dst_static);
            if dst_home.is_none() {
                let deferred = self.state.vector_defer_values.contains(&dest.0);
                use crate::backend::state::SlotAddr;
                if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                    self.state.resolve_slot_addr(dest.0)
                {
                    if !deferred {
                        self.state
                            .emit_fmt(format_args!("    movdqu %xmm0, {}", self.slot_ref(slot.0)));
                    } else {
                        self.state.pending_vec_store = Some((dest.0, "xmm0", false));
                    }
                }
            }
        }
        true
    }

    /// Lane-wise signed dword min/max, 128-bit (`VecSminI32x4`/`VecSmaxI32x4`).
    /// SSE4.1 `pminsd`/`pmaxsd` (the VEX form under AVX2 — memory-foldable
    /// through `emit_sse_binary_128`'s audited branch). x86 producers
    /// (the BB-SLP min/max fold) gate on SSE4.1 exactly like the loop
    /// vectorizer's dword min/max ("the SSE2 baseline lacks dword min/max
    /// entirely — fails closed"); the NEON producer never runs on x86.
    /// Reaching this emitter without SSE4.1 is a producer bug — panic
    /// loudly rather than emit an instruction the CPU cannot decode (the
    /// VecRotlI32x4 immediate-panic discipline).
    fn emit_sminmax_i32x4(&mut self, dest: &Value, args: &[Operand], is_max: bool) {
        assert!(
            self.isa.sse41,
            "VecS{}I32x4 requires SSE4.1 (pminsd/pmaxsd): an x86 producer \
             admitted it without the feature gate",
            if is_max { "max" } else { "min" }
        );
        let inst = if is_max { "pmaxsd" } else { "pminsd" };
        self.emit_sse_binary_128(dest, args, inst);
    }

    pub(super) fn emit_avx_cmp_256(&mut self, dest: &Value, args: &[Operand], inst: &str) {
        assert!(args.len() == 3, "{}: expects lhs, rhs, predicate", inst);
        let imm = match &args[2] {
            Operand::Const(c) => c.to_i64().unwrap_or(0),
            _ => unreachable!("{}: predicate must be a constant immediate", inst),
        };
        self.state.invalidate_vec_peephole();
        let rhs = match self.vec_home_256(&args[1]) {
            Some(reg) => reg,
            None => {
                self.avx_load_arg_to(&args[1], "ymm1");
                "%ymm1".to_string()
            }
        };
        self.avx_load_arg(&args[0]);
        self.state
            .emit_fmt(format_args!("    {} ${}, {}, %ymm0, %ymm0", inst, imm, rhs));
        self.state.vec_last_store_reg = false;
        self.avx_store_dest(dest);
    }

    /// Packed FP compare (SSE2 baseline): `cmpps $imm, src, dst` computes
    /// `dst PRED src` in place; args[0] streams through %xmm0.
    ///
    /// Under AVX2 the THREE-OPERAND VEX form fires first — the same
    /// discipline `emit_int_cmp` got in v6: `vcmpps $imm, %src2, %src1,
    /// %dst` with every operand register-homed and ZERO staging (the
    /// legacy 2-operand form had to copy src1 into the destination
    /// first). The predicate immediates 0/1/2/4 coincide between the
    /// legacy and VEX vocabularies (EQ/LT/LE/NEQ), which are exactly the
    /// C spellings the producers admit.
    pub(super) fn emit_sse_cmp_128(&mut self, dest: &Value, args: &[Operand], inst: &str) {
        assert!(args.len() == 3, "{}: expects lhs, rhs, predicate", inst);
        let imm = match &args[2] {
            Operand::Const(c) => c.to_i64().unwrap_or(0),
            _ => unreachable!("{}: predicate must be a constant immediate", inst),
        };
        // No leading flush: a deferred rhs still held in %xmm0 must keep its
        // cache hit (the load below moves it aside).  Both operands are read
        // through registers only, so no slot can be observed stale.
        self.state.invalidate_vec_peephole();
        if self.avx2_enabled {
            // All-homed fast path: one instruction, no staging. A homed
            // destination may ALIAS a source home — the VEX form reads
            // every source before writing the destination. The homes are
            // the 128-BIT xmm views (this is the SSE-family compare).
            let a = self.vec_home_128(&args[0]);
            let b = self.vec_home_128(&args[1]);
            if let (Some(a), Some(b)) = (&a, &b) {
                if let Some(d) = self.dest_xmm_home_name(dest) {
                    self.state
                        .emit_fmt(format_args!("    v{} ${}, {}, {}, %{}", inst, imm, b, a, d));
                    self.sse_commit_dest_direct(dest, d);
                    return;
                }
            }
            // Staged VEX: rhs (src2) in the reserved second scratch,
            // lhs (src1) through %xmm0 — `vcmpps $imm, %xmm1, %xmm0,
            // %xmm0` computes lhs PRED rhs, overwriting the staged lhs.
            let rhs = match self.vec_home_128(&args[1]) {
                Some(reg) => reg,
                None => {
                    self.sse_load_arg(&args[1], "xmm1");
                    "%xmm1".to_string()
                }
            };
            self.sse_load_arg(&args[0], "xmm0");
            self.state.emit_fmt(format_args!(
                "    v{} ${}, {}, %xmm0, %xmm0",
                inst, imm, rhs
            ));
            self.sse_store_dest(dest, "xmm0");
            return;
        }
        let rhs = match self.vec_home_128(&args[1]) {
            Some(reg) => reg,
            None => {
                self.sse_load_arg(&args[1], "xmm1");
                "%xmm1".to_string()
            }
        };
        self.sse_load_arg(&args[0], "xmm0");
        self.state
            .emit_fmt(format_args!("    {} ${}, {}, %xmm0", inst, imm, rhs));
        self.sse_store_dest(dest, "xmm0");
    }

    /// Packed INTEGER compare on dword lanes (the map vectorizer's
    /// conditional-map path): `dest[i] = (a[i] PRED b[i]) ? -1 : 0` with
    /// `args = [a, b, imm]` and the immediate vocabulary of `VecCmpI32x8`:
    /// 0=eq, 1=lt.s, 2=le.s, 4=ne, 5=lt.u, 6=le.u. `avx2` selects the
    /// 8-lane VEX encoding, else the SSE2 4-lane legacy encoding.
    ///
    /// Both RAW operands are resolved into registers first (the discipline
    /// of the FP compare emitters), so aliasing a==b and deferred-cache
    /// hits stay correct; only then is any biasing applied. The unsigned
    /// predicates XOR 0x80000000 into both operands — a monotone remap of
    /// the unsigned order onto the signed one — with the bias read from a
    /// full-width .rodata constant through vpxor/pxor (both tolerate
    /// unaligned memory). le/ne invert the primary mask by xoring with an
    /// all-ones built in the (then dead) second scratch register.
    pub(super) fn emit_int_cmp(
        &mut self,
        dest: &Value,
        args: &[Operand],
        avx2: bool,
        lane: IntCmpLane,
    ) {
        assert!(args.len() == 3, "int cmp: expects lhs, rhs, predicate");
        let imm = match &args[2] {
            Operand::Const(c) => c.to_i64().unwrap_or(-1),
            _ => unreachable!("int cmp: predicate must be a constant immediate"),
        };
        let unsigned = matches!(imm, 5 | 6);
        let invert = matches!(imm, 2 | 4 | 6);
        let eq = matches!(imm, 0 | 4);
        // Lane-derived spelling.  The dword and byte compares are the SAME
        // algorithm -- only the mnemonic suffix and the sign-bias constant
        // differ -- so they share one implementation instead of two copies
        // that can drift (the unsigned bias is the subtle part: it must be
        // the sign bit OF EACH LANE, i.e. 0x80 replicated per byte, not one
        // 0x8000_0000 word).
        let (sfx, bias_lane, bias_lane_bytes) = match lane {
            IntCmpLane::Dword => ("d", 0x8000_0000u64, 4u8),
            IntCmpLane::Byte => ("b", 0x8080_8080u64, 4u8),
            IntCmpLane::Word => ("w", 0x8000_8000u64, 4u8),
        };
        self.state.invalidate_vec_peephole();
        if avx2 {
            // All-homed three-operand VEX form for the four SIGNED
            // predicates.  x86 provides only `pcmpeq` and `pcmpgt`, so the
            // predicate vocabulary maps as
            //
            //   imm 0  eq     -> vpcmpeq b, a, d
            //   imm 1  lt.s   -> vpcmpgt a, b, d           (d = b > a)
            //   imm 2  le.s   -> vpcmpgt b, a, d ; invert  (d = !(a > b))
            //   imm 4  ne     -> vpcmpeq b, a, d ; invert
            //
            // and the inversion needs an all-ones vector, which is built in
            // the reserved `%ymm1` scratch (the staging path below uses the
            // same register for the same reason, so it is scratch by
            // contract).  Even with the inversion this is three instructions
            // against the staging path's five, and it writes no `%ymm0`.
            //
            // The unsigned forms need a sign-biased COPY of both operands
            // and therefore cannot run register-to-register; they keep the
            // staging path.  (In practice the unsigned range fusion of
            // OP-05c rewrites the common ones into `lt.s` before they ever
            // reach here.)
            //
            // HISTORY -- do not "simplify" this guard.  It was once written
            // `if !unsigned && (eq || !invert)`, which admits `ne`
            // (eq = true, invert = true) and then fell into the `eq` arm and
            // emitted a bare `vpcmpeqd`: `v != 0 ? -v : v` silently
            // computed `v == 0 ? -v : v`.  The regression corpus missed it
            // because no map kernel there used a `!=` mask at AVX2; the
            // differential correctness gate caught it.  `emit_int_cmp` is
            // now covered predicate-by-predicate by
            // `tests/correctness` case `vectorize_cmp_predicate_matrix`.
            if !unsigned {
                if let Some(homes) = self.all_vec_homes_256(&[&args[0], &args[1]], dest) {
                    let (a, b, d) = (&homes[0], &homes[1], &homes[2]);
                    match (eq, invert) {
                        // eq: symmetric, operand order irrelevant.
                        (true, false) => self
                            .state
                            .emit_fmt(format_args!("    vpcmpeq{} {}, {}, {}", sfx, b, a, d)),
                        // lt.s: dst = b > a, and AT&T `vpcmpgt src2, src1, dst`
                        // computes `src1 > src2`, so `b` is the FIRST source.
                        (false, false) => self
                            .state
                            .emit_fmt(format_args!("    vpcmpgt{} {}, {}, {}", sfx, a, b, d)),
                        // ne: !(a == b)
                        (true, true) => self
                            .state
                            .emit_fmt(format_args!("    vpcmpeq{} {}, {}, {}", sfx, b, a, d)),
                        // le.s: !(a > b) -- `a` is the greater-side source.
                        (false, true) => self
                            .state
                            .emit_fmt(format_args!("    vpcmpgt{} {}, {}, {}", sfx, b, a, d)),
                    }
                    if invert {
                        // `vpcmpeqd %r, %r, %r` is the canonical all-ones
                        // idiom and is lane-agnostic (every lane equals
                        // itself at any element width).
                        self.state.emit("    vpcmpeqd %ymm1, %ymm1, %ymm1");
                        self.state
                            .emit_fmt(format_args!("    vpxor %ymm1, {}, {}", d, d));
                    }
                    self.note_vec_dest_in_home(dest, d);
                    return;
                }
            }

            // b: RA home, or staged in the reserved second scratch.
            let b = match self.vec_home_256(&args[1]) {
                Some(reg) => reg,
                None => {
                    self.avx_load_arg_to(&args[1], "ymm1");
                    "%ymm1".to_string()
                }
            };
            self.avx_load_arg(&args[0]); // a -> %ymm0
            // AT&T 3-operand order is `op src2, src1, dst` with
            // dst = src1 OP src2, so the SECOND operand is the GREATER-side
            // source of vpcmpgtd:
            //   lt  wants b > a  ->  `vpcmpgtd a, b, dst` (a in %ymm0 first)
            //   le  wants !(a>b) ->  `vpcmpgtd b, a, dst` (then inverted)
            // eq is commutative and shares either orientation.
            if unsigned {
                let bias = self.lane_const_rip_operand(bias_lane, bias_lane_bytes, 32);
                // a' = a ^ bias in place; b' = b ^ bias into %ymm1 (a home
                // is never written; a staged copy is this emitter's own
                // private temporary, dead after the compare).
                self.state
                    .emit_fmt(format_args!("    vpxor {}, %ymm0, %ymm0", bias));
                self.state
                    .emit_fmt(format_args!("    vpxor {}, {}, %ymm1", bias, b));
                if eq {
                    self.state
                        .emit_fmt(format_args!("    vpcmpeq{} %ymm1, %ymm0, %ymm0", sfx));
                } else if invert {
                    // le.u: !(a' > b')  [src1 = a' = %ymm0]
                    self.state
                        .emit_fmt(format_args!("    vpcmpgt{} %ymm1, %ymm0, %ymm0", sfx));
                } else {
                    // lt.u: b' > a'  [src1 = b' = %ymm1]
                    self.state
                        .emit_fmt(format_args!("    vpcmpgt{} %ymm0, %ymm1, %ymm0", sfx));
                }
            } else if eq {
                self.state
                    .emit_fmt(format_args!("    vpcmpeq{} {}, %ymm0, %ymm0", sfx, b));
            } else if invert {
                // le.s: !(a > b)  [src1 = a = %ymm0]
                self.state
                    .emit_fmt(format_args!("    vpcmpgt{} {}, %ymm0, %ymm0", sfx, b));
            } else {
                // lt.s: b > a  [src1 = b, src2 = a = %ymm0]
                self.state
                    .emit_fmt(format_args!("    vpcmpgt{} %ymm0, {}, %ymm0", sfx, b));
            }
            if invert {
                self.state
                    .emit_fmt(format_args!("    vpcmpeqd %ymm1, %ymm1, %ymm1"));
                self.state
                    .emit_fmt(format_args!("    vpxor %ymm1, %ymm0, %ymm0"));
            }
            self.state.vec_last_store_reg = false;
            self.avx_store_dest(dest);
        } else {
            // SSE2 legacy two-operand forms (`op src, dst` = dst OP src):
            // the destination register must carry the GREATER-side operand
            // of pcmpgtd — b for lt (b > a), a for le (a > b, then the
            // inversion). eq is commutative. The unsigned bias uses pxor's
            // memory form (unaligned-safe); a homed operand is copied to
            // %xmm1 because the home must not be clobbered.
            let lt_loads_b_first = eq || !invert;
            let (dst_arg, src_arg) = if lt_loads_b_first {
                (&args[1], &args[0]) // b -> %xmm0, a homed/staged
            } else {
                (&args[0], &args[1]) // a -> %xmm0, b homed/staged
            };
            let src = match self.vec_home_128(src_arg) {
                Some(reg) => reg,
                None => {
                    self.sse_load_arg(src_arg, "xmm1");
                    "%xmm1".to_string()
                }
            };
            self.sse_load_arg(dst_arg, "xmm0");
            if unsigned {
                let bias = self.lane_const_rip_operand(bias_lane, bias_lane_bytes, 16);
                // Bias both: the dst in place, the src into %xmm1.
                self.state
                    .emit_fmt(format_args!("    pxor {}, %xmm0", bias));
                if src == "%xmm1" {
                    self.state
                        .emit_fmt(format_args!("    pxor {}, %xmm1", bias));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movdqa {}, %xmm1", src));
                    self.state
                        .emit_fmt(format_args!("    pxor {}, %xmm1", bias));
                }
                let mn = if eq {
                    format!("pcmpeq{sfx}")
                } else {
                    format!("pcmpgt{sfx}")
                };
                self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", mn));
            } else {
                let mn = if eq {
                    format!("pcmpeq{sfx}")
                } else {
                    format!("pcmpgt{sfx}")
                };
                // %xmm0 holds the greater-side operand (b for lt, a for
                // le); the src is the other side: dst > src.
                self.state
                    .emit_fmt(format_args!("    {} {}, %xmm0", mn, src));
            }
            if invert {
                // `pcmpeqd %r, %r` is the canonical all-ones idiom and is
                // lane-agnostic: every lane compares equal to itself, so the
                // result is all-ones whatever the element width.  No byte
                // spelling is needed (and `pcmpeqd` is the form the
                // microarchitecture recognises as a zeroing/ones idiom).
                self.state
                    .emit_fmt(format_args!("    pcmpeqd %xmm1, %xmm1"));
                self.state.emit_fmt(format_args!("    pxor %xmm1, %xmm0"));
            }
            self.sse_store_dest(dest, "xmm0");
        }
    }
    /// Packed byte-lane compare (VecCmpI8x32): the byte-granularity mirror
    /// of `emit_int_cmp`'s AVX2 branch — same predicate-immediate
    /// vocabulary, same operand orientation and inversion discipline, with
    /// `vpcmpgtb`/`vpcmpeqb` and a per-byte `0x80` bias for the unsigned
    /// forms (xor with 0x80 is a monotone map on the unsigned byte order,
    /// turning unsigned compares into signed ones the hardware provides).
    pub(super) fn emit_int_cmp_i8(&mut self, dest: &Value, args: &[Operand], avx2: bool) {
        assert!(args.len() == 3, "i8 cmp: expects lhs, rhs, predicate");
        let imm = match &args[2] {
            Operand::Const(c) => c.to_i64().unwrap_or(-1),
            _ => unreachable!("i8 cmp: predicate must be a constant immediate"),
        };
        let unsigned = matches!(imm, 5 | 6);
        let invert = matches!(imm, 2 | 4 | 6);
        let eq = matches!(imm, 0 | 4);
        self.state.invalidate_vec_peephole();
        // All-homed fast path for eq / signed lt (the fused range mask's
        // predicate): one vpcmpeqb/vpcmpgtb, zero staging.
        if !unsigned && (eq || !invert) {
            if let Some(homes) = self.all_vec_homes_256(&[&args[0], &args[1]], dest) {
                let (a, b, d) = (&homes[0], &homes[1], &homes[2]);
                if eq {
                    self.state
                        .emit_fmt(format_args!("    vpcmpeqb {}, {}, {}", b, a, d));
                } else {
                    self.state
                        .emit_fmt(format_args!("    vpcmpgtb {}, {}, {}", a, b, d));
                }
                self.note_vec_dest_in_home(dest, d);
                return;
            }
        }
        if !avx2 {
            self.emit_int_cmp_i8_sse2(dest, args, unsigned, invert, eq);
            return;
        }
        // b: RA home, or staged in the reserved second scratch.
        let b = match self.vec_home_256(&args[1]) {
            Some(reg) => reg,
            None => {
                self.avx_load_arg_to(&args[1], "ymm1");
                "%ymm1".to_string()
            }
        };
        self.avx_load_arg(&args[0]); // a -> %ymm0
        // Same AT&T orientation contract as the dword emitter:
        //   lt  wants b > a  ->  `vpcmpgtb a, b, dst` (a in %ymm0 first)
        //   le  wants !(a>b) ->  `vpcmpgtb b, a, dst` (then inverted)
        if unsigned {
            let bias = self.lane_const_rip_operand(0x80, 1, 32);
            self.state
                .emit_fmt(format_args!("    vpxor {}, %ymm0, %ymm0", bias));
            self.state
                .emit_fmt(format_args!("    vpxor {}, {}, %ymm1", bias, b));
            if eq {
                self.state
                    .emit_fmt(format_args!("    vpcmpeqb %ymm1, %ymm0, %ymm0"));
            } else if invert {
                // le.u: !(a' > b')  [src1 = a' = %ymm0]
                self.state
                    .emit_fmt(format_args!("    vpcmpgtb %ymm1, %ymm0, %ymm0"));
            } else {
                // lt.u: b' > a'  [src1 = b' = %ymm1]
                self.state
                    .emit_fmt(format_args!("    vpcmpgtb %ymm0, %ymm1, %ymm0"));
            }
        } else if eq {
            self.state
                .emit_fmt(format_args!("    vpcmpeqb {}, %ymm0, %ymm0", b));
        } else if invert {
            // le.s: !(a > b)  [src1 = a = %ymm0]
            self.state
                .emit_fmt(format_args!("    vpcmpgtb {}, %ymm0, %ymm0", b));
        } else {
            // lt.s: b > a  [src1 = b, src2 = a = %ymm0]
            self.state
                .emit_fmt(format_args!("    vpcmpgtb %ymm0, {}, %ymm0", b));
        }
        if invert {
            self.state
                .emit_fmt(format_args!("    vpcmpeqb %ymm1, %ymm1, %ymm1"));
            self.state
                .emit_fmt(format_args!("    vpxor %ymm1, %ymm0, %ymm0"));
        }
        self.state.vec_last_store_reg = false;
        self.avx_store_dest(dest);
    }

    /// SSE2-baseline 128-bit byte compare (VecCmpI8x16): the exact twin of
    /// `emit_int_cmp`'s SSE branch with the `b` mnemonic suffix —
    /// `pcmpeqb`/`pcmpgtb` are baseline SSE2, so the whole classifier
    /// lowering (fused `vpaddb`-shaped `paddb` + compare) survives on the
    /// GCC-exact `-march=x86-64` target with no SSSE3 dependency.
    fn emit_int_cmp_i8_sse2(
        &mut self,
        dest: &Value,
        args: &[Operand],
        unsigned: bool,
        invert: bool,
        eq: bool,
    ) {
        // EXACT mirror of `emit_int_cmp`'s SSE2 branch with the `b`
        // suffix: the destination register carries the GREATER-side
        // operand of pcmpgtb — b for lt (b > a), a for le (a > b, then
        // the inversion). eq is commutative. The unsigned bias is the
        // per-byte 0x80 XOR (monotone on the unsigned byte order, so it
        // turns unsigned compares into the signed ones the hardware
        // provides); a homed operand is copied to %xmm1 because the home
        // must not be clobbered.
        let lt_loads_b_first = eq || !invert;
        let (dst_arg, src_arg) = if lt_loads_b_first {
            (&args[1], &args[0]) // b -> %xmm0, a homed/staged
        } else {
            (&args[0], &args[1]) // a -> %xmm0, b homed/staged
        };
        let src = match self.vec_home_128(src_arg) {
            Some(reg) => reg,
            None => {
                self.sse_load_arg(src_arg, "xmm1");
                "%xmm1".to_string()
            }
        };
        self.sse_load_arg(dst_arg, "xmm0");
        if unsigned {
            // Bias both: the dst in place, the src into %xmm1.
            let bias = self.lane_const_rip_operand(0x80, 1, 16);
            self.state
                .emit_fmt(format_args!("    pxor {}, %xmm0", bias));
            if src == "%xmm1" {
                self.state
                    .emit_fmt(format_args!("    pxor {}, %xmm1", bias));
            } else {
                self.state
                    .emit_fmt(format_args!("    movdqa {}, %xmm1", src));
                self.state
                    .emit_fmt(format_args!("    pxor {}, %xmm1", bias));
            }
            let mn = if eq { "pcmpeqb" } else { "pcmpgtb" };
            self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", mn));
        } else {
            let mn = if eq { "pcmpeqb" } else { "pcmpgtb" };
            // %xmm0 holds the greater-side operand (b for lt, a for
            // le); the src is the other side: dst > src.
            self.state
                .emit_fmt(format_args!("    {} {}, %xmm0", mn, src));
        }
        if invert {
            // `pcmpeqd %r, %r` is the canonical all-ones idiom and is
            // lane-agnostic — every lane compares equal to itself.
            self.state.emit("    pcmpeqd %xmm1, %xmm1");
            self.state.emit("    pxor %xmm1, %xmm0");
        }
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest, "xmm0");
    }

    pub(super) fn emit_int_rotl_i32x4(&mut self, dest: &Value, args: &[Operand]) {
        assert!(args.len() == 2, "i32x4 rotl: expects v, amount");
        let amt = match &args[1] {
            Operand::Const(c) => c.to_i64().unwrap_or(-1),
            _ => unreachable!("i32x4 rotl: amount must be a constant immediate"),
        };
        assert!(
            (1..=31).contains(&amt),
            "i32x4 rotl: rotate amount out of range"
        );
        self.state.invalidate_vec_peephole();
        // Dest-homed fast path: bring the input into the destination home
        // (a no-op when the RA coalesced the dying input onto it), then
        // pslld via %xmm0 and psrld in place —
        //   movdqa %t, %xmm0; pslld $a, %xmm0; psrld $(32-a), %t; por.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(dest_reg) {
                if let Operand::Value(src_v) = &args[0] {
                    if let Some(src) = self.vec_operand_reg(src_v) {
                        let target = phys_reg_name(dest_reg);
                        // %xmm0 is clobbered below: flush any pending
                        // deferred store first, unless it is the source we
                        // consume by moving it into the target.
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(src_v.0) {
                            self.state.pending_vec_store = None;
                        } else {
                            self.flush_pending_vec_store_impl();
                        }
                        if src != target {
                            self.state
                                .emit_fmt(format_args!("    movdqa %{}, %{}", src, target));
                        }
                        self.state
                            .emit_fmt(format_args!("    movdqa %{}, %xmm0", target));
                        self.state
                            .emit_fmt(format_args!("    pslld ${}, %xmm0", amt));
                        self.state
                            .emit_fmt(format_args!("    psrld ${}, %{}", 32 - amt, target));
                        self.state
                            .emit_fmt(format_args!("    por %xmm0, %{}", target));
                        self.sse_mark_in_place(dest, target);
                        return;
                    }
                }
            }
        }
        self.sse_load_arg(&args[0], "xmm0");
        self.state.emit("    movdqa %xmm0, %xmm1");
        self.state
            .emit_fmt(format_args!("    pslld ${}, %xmm0", amt));
        self.state
            .emit_fmt(format_args!("    psrld ${}, %xmm1", 32 - amt));
        self.state.emit("    por %xmm1, %xmm0");
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest, "xmm0");
    }

    /// ARX lane shuffle (VecShufdI32x4): `dest[i] = v[imm & 3 >> 2i]`,
    /// `args = [v, Const(imm)]` — one pshufd (SSE2).  The transform's
    /// lane-rotation between quarter-round groups.
    pub(super) fn emit_int_shufd_i32x4(&mut self, dest: &Value, args: &[Operand]) {
        assert!(args.len() == 2, "i32x4 shufd: expects v, imm");
        let imm = match &args[1] {
            Operand::Const(c) => c.to_i64().unwrap_or(-1),
            _ => unreachable!("i32x4 shufd: imm must be a constant immediate"),
        };
        assert!((0..=255).contains(&imm), "i32x4 shufd: imm8 out of range");
        self.state.invalidate_vec_peephole();
        // Dest-homed fast path: pshufd is a genuine three-operand form, so
        // `pshufd $imm, %src, %dst` needs no staging even across registers
        // — and when the RA coalesced a dying source onto the destination
        // the shuffle is fully in-place.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(dest_reg) {
                if let Operand::Value(src_v) = &args[0] {
                    if let Some(src) = self.vec_operand_reg(src_v) {
                        let target = phys_reg_name(dest_reg);
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(src_v.0) {
                            // The source's pending deferred store (single
                            // use) flowed through the register.
                            self.state.pending_vec_store = None;
                        }
                        self.state
                            .emit_fmt(format_args!("    pshufd ${}, %{}, %{}", imm, src, target));
                        self.sse_mark_in_place(dest, target);
                        return;
                    }
                }
            }
        }
        // A register-homed source is the legacy 3-operand form's src
        // directly (`pshufd $imm, %home, %xmm0`); otherwise load to %xmm0
        // and shuffle in place.
        if let Some(home) = self.vec_home_128(&args[0]) {
            self.flush_pending_vec_store_impl();
            self.state
                // `home` already carries its own '%' (vec_home_128).
                .emit_fmt(format_args!("    pshufd ${}, {}, %xmm0", imm, home));
        } else {
            self.sse_load_arg(&args[0], "xmm0");
            self.state
                .emit_fmt(format_args!("    pshufd ${}, %xmm0, %xmm0", imm));
        }
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest, "xmm0");
    }

    /// ARX byte shuffle (VecShufbI32x4): `dest byte i = v byte mask[i]`,
    /// `args = [v, mask]` — one pshufb (SSSE3; the transform only emits the
    /// op under SSSE3).  Covers dword rotates by whole bytes and fused
    /// rotate+lane-permute masks (the ARX transform's composition).
    pub(super) fn emit_int_shufb_i32x4(&mut self, dest: &Value, args: &[Operand]) {
        assert!(args.len() == 2, "i32x4 shufb: expects v, mask");
        self.state.invalidate_vec_peephole();
        // Dest-homed fast path: bring the data into the destination home
        // (a no-op when the RA coalesced the dying input onto it) and
        // shuffle in place with a resolvable mask register.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(dest_reg) {
                if let (Operand::Value(data_v), Operand::Value(mask_v)) = (&args[0], &args[1]) {
                    if let (Some(data), Some(mask)) =
                        (self.vec_operand_reg(data_v), self.vec_operand_reg(mask_v))
                    {
                        let target = phys_reg_name(dest_reg);
                        if mask != target {
                            if data != target {
                                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(data_v.0)
                                {
                                    self.state.pending_vec_store = None;
                                }
                                self.state
                                    .emit_fmt(format_args!("    movdqa %{}, %{}", data, target));
                            } else if self.state.pending_vec_store.map(|(p, _, _)| p)
                                == Some(data_v.0)
                            {
                                self.state.pending_vec_store = None;
                            }
                            if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(mask_v.0) {
                                self.state.pending_vec_store = None;
                            }
                            self.state
                                .emit_fmt(format_args!("    pshufb %{}, %{}", mask, target));
                            self.sse_mark_in_place(dest, target);
                            return;
                        }
                    }
                }
            }
        }
        self.sse_load_arg(&args[0], "xmm0");
        // The mask rides a register home when it kept one (the canonical
        // loop-invariant preheader-built shape); otherwise through xmm1.
        if let Some(home) = self.vec_home_128(&args[1]) {
            self.flush_pending_vec_store_impl();
            self.state
                // `home` already carries its own '%' (vec_home_128).
                .emit_fmt(format_args!("    pshufb {}, %xmm0", home));
        } else {
            self.sse_load_arg(&args[1], "xmm1");
            self.state.emit("    pshufb %xmm1, %xmm0");
        }
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest, "xmm0");
    }

    /// BB-SLP packed lane shift by a uniform immediate (128-bit):
    /// `dest = <inst> $amount, src` for `psllw/psrlw/psraw/pslld/psrld/
    /// psrad/psllq/psrlq`. `args = [vector, Const(amount)]`; the amount's
    /// defined range [1, lane_bits-1] is enforced by the SLP pack builder
    /// (the only producer), so the assert here is a compiler-bug tripwire.
    ///
    /// Register-home aware: under AVX the VEX.128 three-operand immediate
    /// form `v<inst> $imm, %src, %dst` needs no staging at all (homed
    /// sources read in place, unhomed stream through %xmm0 with the
    /// ordinary deferred-store chain); the SSE2-only path stages through
    /// the destination home (movdqa + the destructive immediate shift)
    /// or through %xmm0.
    pub(super) fn emit_sse_shift_imm_128(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        sse_inst: &str,
        max_amount: i64,
    ) {
        assert!(
            args.len() == 2,
            "emit_sse_shift_imm_128: malformed intrinsic {} ({} args)",
            sse_inst,
            args.len()
        );
        let amount = self.operand_to_imm_i64(&args[1]);
        assert!(
            (1..=max_amount).contains(&amount),
            "emit_sse_shift_imm_128: shift amount {amount} outside the defined range 1..={max_amount}"
        );
        self.state.invalidate_vec_peephole();
        if self.avx2_enabled {
            // NO VLFOLD here, ever: the VEX.128 immediate-shift encoding is
            // REGISTER-ONLY (the memory form exists solely under EVEX /
            // AVX-512 — see the ISA NOTE on `memfold_consumer_128`). The
            // analysis never admits a shift consumer, and the safety net
            // materialises any pending fold before this emitter runs.
            //
            // VEX.128 three-operand immediate form: `v<inst> $imm, %src,
            // %dst` — one instruction for every homed shape (the single
            // scratch cannot collide with itself, unlike the binary VEX
            // path's src1/src2 pair).
            let src = self.vex128_source(&args[0], "xmm0");
            let dst_home = self.dest_xmm_home_name(dest_ptr);
            let dst = match dst_home {
                Some(name) => format!("%{}", name),
                None => "%xmm0".to_string(),
            };
            self.state.emit_fmt(format_args!(
                "    v{} ${}, {}, {}",
                sse_inst, amount, src, dst
            ));
            // Commit BEFORE creating the dest's own deferred store: the
            // commit clears a pending naming the committed value (a SOURCE
            // pending consumed by this write); the dest's own pending must
            // survive it (the hsum stale-slot miscompile class).
            let dst_static: &'static str = dst_home.unwrap_or("xmm0");
            self.sse_commit_dest_direct(dest_ptr, dst_static);
            if dst_home.is_none() {
                let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
                use crate::backend::state::SlotAddr;
                if let Some(crate::backend::state::SlotAddr::Direct(slot)) =
                    self.state.resolve_slot_addr(dest_ptr.0)
                {
                    if !deferred {
                        self.state
                            .emit_fmt(format_args!("    movdqu %xmm0, {}", self.slot_ref(slot.0)));
                    } else {
                        self.state.pending_vec_store = Some((dest_ptr.0, "xmm0", false));
                    }
                }
            }
            return;
        }
        // SSE2-only: dest-homed in-place form. Bring the source into the
        // destination home (a no-op when the RA coalesced the dying source
        // onto it), then the destructive `inst $imm, %dst`.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(dest_reg) {
                if let Operand::Value(src_v) = &args[0] {
                    if let Some(src) = self.vec_operand_reg(src_v) {
                        let target = phys_reg_name(dest_reg);
                        // %xmm0-adjacent staging below could clobber a
                        // pending deferred store: consume it when it is
                        // the source, flush it otherwise.
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(src_v.0) {
                            self.state.pending_vec_store = None;
                        } else {
                            self.flush_pending_vec_store_impl();
                        }
                        if src != target {
                            self.state
                                .emit_fmt(format_args!("    movdqa %{}, %{}", src, target));
                        }
                        self.state
                            .emit_fmt(format_args!("    {} ${}, %{}", sse_inst, amount, target));
                        self.sse_mark_in_place(dest_ptr, target);
                        return;
                    }
                }
            }
        }
        // Held in %xmm0 (last-stored/deferred streaming shape — e.g. a
        // VDEFER'd VecLoad whose slot was never written): shift in place
        // and consume the pending store, exactly like the binary VEX
        // path's deferred handling. Without this the staged fallback
        // re-materialised the value from its SOURCE MEMORY — a duplicate
        // load of the vector.
        if matches!(&args[0], Operand::Value(v)
            if self.state.sse_last_store_reg
                && self.state.sse_last_store_val == Some(v.0)
                && self.state.sse_last_store_reg_name == Some("xmm0"))
        {
            if let Operand::Value(v) = &args[0] {
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.state.pending_vec_store = None;
                }
            }
            self.state
                .emit_fmt(format_args!("    v{} ${}, %xmm0, %xmm0", sse_inst, amount));
            self.state.sse_last_store_reg = false;
            self.sse_store_dest(dest_ptr, "xmm0");
            return;
        }
        self.sse_load_arg(&args[0], "xmm0");
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, amount));
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// BB-SLP packed lane shift by a uniform immediate (256-bit):
    /// `dest = v<inst> $amount, src` for `vpsllw/vpsrlw/vpsraw/vpslld/
    /// vpsrld/vpsrad/vpsllq/vpsrlq` — the VEX immediate forms are genuine
    /// three-operand (r/m256 source, ymm destination), so a memfold-deferred
    /// load fuses into the shift (`v<inst> $imm, MEM, %dst`) and homed
    /// chains compute register-to-register with zero staging.
    pub(super) fn emit_avx_shift_imm_256(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        avx_inst: &str,
        max_amount: i64,
    ) {
        assert!(
            args.len() == 2,
            "emit_avx_shift_imm_256: malformed intrinsic {} ({} args)",
            avx_inst,
            args.len()
        );
        let amount = self.operand_to_imm_i64(&args[1]);
        assert!(
            (1..=max_amount).contains(&amount),
            "emit_avx_shift_imm_256: shift amount {amount} outside the defined range 1..={max_amount}"
        );
        self.state.invalidate_vec_peephole();
        self.state.dirty_upper_ymm = true;
        // NO VLFOLD here, ever: the VEX.256 immediate-shift encoding is
        // REGISTER-ONLY (the memory form exists solely under EVEX /
        // AVX-512 — see the ISA NOTE on `memfold_consumer_128`). The
        // analysis never admits a shift consumer, and the safety net
        // materialises any pending fold before this emitter runs.
        //
        // All-homed fast path: `v<inst> $imm, %ymmS, %ymmD` with zero
        // staging (defer-overflow promoted chains).
        if let Operand::Value(v) = &args[0] {
            if let Some(&r0) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(r0) {
                    if let Some(&rd) = self.reg_assignments.get(&dest_ptr.0) {
                        if is_xmm_reg(rd) {
                            let n0 = phys_reg_name_256(r0);
                            let nd = phys_reg_name_256(rd);
                            self.state.emit_fmt(format_args!(
                                "    {} ${}, %{}, %{}",
                                avx_inst, amount, n0, nd
                            ));
                            self.state.dirty_upper_ymm = true;
                            self.state.vec_claim_live_reg(dest_ptr.0, nd);
                            self.state.vec_last_store_val = Some(dest_ptr.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(nd);
                            self.state.reg_cache.invalidate_acc();
                            // The consumed source's pending deferred store
                            // (single-use by construction) flowed through
                            // its register.
                            if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                                self.state.pending_vec_store = None;
                            }
                            return;
                        }
                    }
                }
            }
        }
        // Held in %ymm0 (last-stored/deferred streaming shape): shift in
        // place; the value flows into the result through %ymm0.
        if matches!(&args[0], Operand::Value(v)
            if self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(v.0)
                && self.state.vec_last_store_reg_name == Some("ymm0"))
        {
            if let Operand::Value(v) = &args[0] {
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.state.pending_vec_store = None;
                }
            }
            self.state
                .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", avx_inst, amount));
            self.state.vec_last_store_reg = false;
            self.avx_store_dest(dest_ptr);
            return;
        }
        // Staged fallback: load (register cache / slot / memory) into
        // %ymm0, shift in place, store.
        self.avx_load_arg(&args[0]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", avx_inst, amount));
        self.state.vec_last_store_reg = false;
        self.avx_store_dest(dest_ptr);
    }

    /// ARX pack (VecPackI32x4): four scalar u32 values into one 4×I32
    /// vector, `args = [x0, x1, x2, x3]` (lane order).  Two movd/punpckldq
    /// pairs and a punpcklqdq — xmm0/xmm1 only, no third scratch:
    ///   xmm0 = [x0, x1] (punpckldq), xmm1 = [x2, x3] (movq of the packed
    ///   GP pair), xmm0 = [x0, x1, x2, x3] (punpcklqdq).
    ///
    /// All-constant fast path: every ARX vectorizer materialises its
    /// loop-invariant pshufb rotate masks through this pack with four
    /// `IrConst::I32` lanes, and SLP packs of constants land here too.
    /// Building such a mask through the GPR staging dance below costs
    /// eleven instructions (and the rax/rcx scratch pair) PER MASK, once
    /// per CALL — the chacha20_block kernel runs 2M calls, so the
    /// prologue cost dominates.  Fold the four lanes into the shared
    /// `.LCVEC` const pool (deduplicated, 16-byte aligned, so the aligned
    /// load is legal on every profile) and load it in ONE instruction —
    /// the exact shape ICX emits for its rotate masks.  The prologue
    /// shrink (22 staging instructions → 2 loads in chacha20_core,
    /// verified in emitted asm) plus the chain-homing change (§1b of
    /// PERF-PROVENANCE-S49) carry the measured default-path delta;
    /// per-change isolation needs a dedicated A/B build, not prose.
    pub(super) fn emit_int_pack_i32x4(&mut self, dest: &Value, args: &[Operand]) {
        assert!(args.len() == 4, "i32x4 pack: expects four scalars");
        self.state.invalidate_vec_peephole();
        self.flush_pending_vec_store_impl();
        let mut lanes = [0u32; 4];
        let all_const = args.iter().enumerate().all(|(i, a)| match a {
            Operand::Const(c) => match c.to_i64() {
                Some(v) => {
                    lanes[i] = v as u32;
                    true
                }
                None => false,
            },
            _ => false,
        });
        if all_const {
            let mut bytes = [0u8; 16];
            for (i, l) in lanes.iter().enumerate() {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&l.to_le_bytes());
            }
            let label = self.state.get_vec_const_label(&bytes);
            // VEX.128 aligned load under AVX2+, the legacy aligned form on
            // the SSE2 baseline — the pool's per-entry `.p2align 4`
            // satisfies both.  Dest-homed values load straight into their
            // home (zero staging); unhomed values stream through %xmm0
            // into the dest's slot exactly like the staging path's tail.
            match self.dest_xmm_home_name(dest) {
                Some(name) => {
                    if self.avx2_enabled {
                        self.state
                            .emit_fmt(format_args!("    vmovdqa {}(%rip), %{}", label, name));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movdqa {}(%rip), %{}", label, name));
                    }
                    self.sse_commit_dest_direct(dest, name);
                }
                None => {
                    if self.avx2_enabled {
                        self.state
                            .emit_fmt(format_args!("    vmovdqa {}(%rip), %xmm0", label));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movdqa {}(%rip), %xmm0", label));
                    }
                    self.state.sse_last_store_reg = false;
                    self.sse_store_dest(dest, "xmm0");
                }
            }
            return;
        }
        self.operand_to_reg(&args[0], "rax");
        self.state.emit("    movd %eax, %xmm0");
        self.operand_to_reg(&args[1], "rcx");
        self.state.emit("    movd %ecx, %xmm1");
        self.state.emit("    punpckldq %xmm1, %xmm0"); // xmm0 = [x0, x1, ..]
        self.operand_to_reg(&args[2], "rax");
        self.operand_to_reg(&args[3], "rcx");
        self.state.emit("    shlq $32, %rcx");
        self.state.emit("    orq %rcx, %rax"); // rax = x2 | (x3 << 32)
        self.state.emit("    movq %rax, %xmm1"); // xmm1 = [x2, x3, 0, 0]
        self.state.emit("    punpcklqdq %xmm1, %xmm0"); // [x0..x3]
        self.state.sse_last_store_reg = false;
        self.sse_store_dest(dest, "xmm0");
    }

    /// ARX lane extract (VecExtractLaneI32x4): `dest = v[Const(lane)]`,
    /// `args = [v, Const(lane)]`, dest = scalar u32.  pextrd under
    /// SSE4.1 (1 op); pshufd-to-lane-0 + movd on the SSE2 baseline.
    pub(super) fn emit_int_extract_i32x4(&mut self, dest: Option<&Value>, args: &[Operand]) {
        assert!(args.len() == 2, "i32x4 extract: expects v, lane");
        let lane = match &args[1] {
            Operand::Const(c) => c.to_i64().unwrap_or(-1),
            _ => unreachable!("i32x4 extract: lane must be a constant immediate"),
        };
        assert!((0..=3).contains(&lane), "i32x4 extract: lane out of range");
        let Some(dest) = dest else {
            return;
        };
        self.state.invalidate_vec_peephole();
        // Fast path: a homed/held source extracts directly from its
        // register — no staging through %xmm0.  (SSE4.1 pextrd reads any
        // XMM source; the SSE2 fallback still needs the shuffle, but a
        // register source keeps the pshufd three-operand form.)
        if let Operand::Value(src_v) = &args[0] {
            if let Some(src) = self.vec_operand_reg(src_v) {
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(src_v.0) {
                    // Single-use deferred value consumed via register.
                    self.state.pending_vec_store = None;
                }
                if self.isa.sse41 {
                    self.state
                        .emit_fmt(format_args!("    pextrd ${}, %{}, %eax", lane, src));
                    self.store_rax_to(dest);
                    return;
                }
                if lane == 0 {
                    self.state.emit_fmt(format_args!("    movd %{}, %eax", src));
                    self.store_rax_to(dest);
                    return;
                }
                // imm nibble i = lane: select lane into dword 0, straight
                // from the source register (three-operand pshufd).
                let imm = lane | (lane << 2) | (lane << 4) | (lane << 6);
                self.state
                    .emit_fmt(format_args!("    pshufd ${}, %{}, %xmm0", imm, src));
                self.state.emit("    movd %xmm0, %eax");
                self.store_rax_to(dest);
                return;
            }
        }
        self.sse_load_arg(&args[0], "xmm0");
        if self.isa.sse41 {
            self.state
                .emit_fmt(format_args!("    pextrd ${}, %xmm0, %eax", lane));
        } else {
            if lane != 0 {
                // imm nibble i = lane: select lane into dword 0.
                let imm = lane | (lane << 2) | (lane << 4) | (lane << 6);
                self.state
                    .emit_fmt(format_args!("    pshufd ${}, %xmm0, %xmm0", imm));
            }
            self.state.emit("    movd %xmm0, %eax");
        }
        self.store_rax_to(dest);
    }

    /// Lane-mask select (AVX): `args = [false_vec, true_vec, mask]`,
    /// `vblendvps mask, true(reg/mem), false(reg), dst`.  The false vector
    /// streams through %ymm0; the mask and the true vector are read from
    /// their homes when register-allocated, otherwise at most ONE of them is
    /// copied through the reserved %ymm1 and the other is read as the memory
    /// source (legal in the src2 position).  No RA home (%ymm2..%ymm15) is
    /// ever written.
    pub(super) fn emit_avx_blendv_256(&mut self, dest: &Value, args: &[Operand], inst: &str) {
        assert!(args.len() == 3, "{}: expects false, true, mask", inst);
        self.state.invalidate_vec_peephole();
        // Degenerate `mask ? x : x` — the blend is the identity on x.
        if matches!((&args[0], &args[1]), (Operand::Value(a), Operand::Value(b)) if a == b) {
            self.avx_load_arg(&args[0]);
            self.state.vec_last_store_reg = false;
            self.avx_store_dest(dest);
            return;
        }
        let mask_home = self.vec_home_256(&args[2]);
        let true_home = self.vec_home_256(&args[1]);
        // All-homed three-operand VEX form: `blendv %mask, %true, %false, %dst`
        // with zero staging.  This is the shape the conditional-map vectorizer
        // produces once its compare and both arms are register-allocated; the
        // generic path below would spend a `vmovdqa` moving the false arm into
        // %ymm0 and another moving the result back out, i.e. two register
        // copies per loop iteration on the critical path.
        if let Some(homes) = self.all_vec_homes_256(&[&args[0], &args[1], &args[2]], dest) {
            let (f, t, m, d) = (&homes[0], &homes[1], &homes[2], &homes[3]);
            self.state
                .emit_fmt(format_args!("    {} {}, {}, {}, {}", inst, m, t, f, d));
            self.note_vec_dest_in_home(dest, d);
            return;
        }
        // The false vector streams through %ymm0 (src1 of vblendv) and is
        // loaded exactly once per arm.  Its load is also what commits any
        // unrelated pending deferred store before a memory source below
        // could go stale (the deferred-store window rules guarantee at most
        // one pending value at a time, so the mask move in the (None, None)
        // arm consumes it before the false load ever runs).
        let (mask, tval) = match (mask_home, true_home) {
            (Some(m), Some(t)) => {
                self.avx_load_arg(&args[0]);
                (m, t)
            }
            (Some(m), None) => {
                self.avx_load_arg(&args[0]);
                self.avx_load_arg_to(&args[1], "ymm1");
                (m, "%ymm1".to_string())
            }
            (None, Some(t)) => {
                // Resolve the MASK first (same rationale as the (None, None)
                // arm): when the mask is the deferred cmp result still held
                // in %ymm0, moving it aside now keeps the cache hit —
                // loading the false vector first would flush the deferred
                // mask to its (never-written) slot and re-read it: one dead
                // store + one stack load per iteration.  Both orders are
                // sound (the false load flushes whatever the mask move did
                // not commit); mask-first is simply the order the canonical
                // cmp→blendv chain needs.
                self.avx_load_arg_to(&args[2], "ymm1");
                self.avx_load_arg(&args[0]);
                ("%ymm1".to_string(), t)
            }
            (None, None) => {
                // Neither the mask nor the true vector is register-homed.
                // Resolve the MASK first: the canonical shape is a deferred
                // cmp result flowing straight into this blend, so the mask
                // is still held in %ymm0 — moving it aside BEFORE the false
                // vector's load keeps the cache hit (the false load would
                // otherwise flush the deferred mask to its slot and re-read
                // it: one dead store + one stack load per iteration).  The
                // true vector is then the src2 memory source, legal in that
                // position; its slot is fresh because the false load has
                // committed whatever the mask move did not.
                self.avx_load_arg_to(&args[2], "ymm1");
                self.avx_load_arg(&args[0]);
                let t = self.vec_mem_source_after_flush(&args[1], inst);
                ("%ymm1".to_string(), t)
            }
        };
        let dest_home = self
            .reg_assignments
            .get(&dest.0)
            .copied()
            .filter(|r| is_xmm_reg(*r))
            .map(phys_reg_name_256);
        let dst = dest_home
            .map(|n| format!("%{}", n))
            .unwrap_or_else(|| "%ymm0".to_string());
        self.state.emit_fmt(format_args!(
            "    {} {}, {}, %ymm0, {}",
            inst, mask, tval, dst
        ));
        self.state.dirty_upper_ymm = true;
        match dest_home {
            Some(name) => {
                self.state.vec_claim_live_reg(dest.0, name);
                self.state.vec_last_store_val = Some(dest.0);
                self.state.vec_last_store_reg = true;
                self.state.vec_last_store_reg_name = Some(name);
                self.state.reg_cache.invalidate_acc();
            }
            None => {
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
        }
    }

    /// Lane-mask select (SSE2 baseline, no SSE4.1 `blendvps` dependency):
    /// `dst = (true & mask) | (false & ~mask)` as
    ///   xmm1 = mask; xmm0 = true; andps xmm1, xmm0; andnps false, xmm1;
    ///   orps xmm1, xmm0
    /// The false vector is read from its home / slot / fold as the
    /// `andnps` memory or register source, so only %xmm0/%xmm1 are written.
    ///
    /// `domain` picks the mnemonic triple: PS/PD for the FP lanes (legacy
    /// encodings, domain-aligned with the payload) and INT for integer
    /// payloads (pand/pandn/por — integer domain, no float-bypass delay).
    pub(super) fn emit_sse_blendv_128(
        &mut self,
        dest: &Value,
        args: &[Operand],
        domain: BlendvDomain,
    ) {
        assert!(args.len() == 3, "blendv128: expects false, true, mask");
        // SSE4.1+AVX2 takes the ONE-instruction VEX lane select first.
        // The FP compares produce per-DWORD masks (cmpps) or per-QWORD
        // masks (cmppd, whose two dwords agree), so the dword-granular
        // vblendvps/vblendvpd sign select is exact for both FP widths.
        // Legacy SSE4.1-only targets keep the 3-op andps/andnps/orps
        // select: blendvps carries its mask IMPLICITLY in %xmm0 there
        // (a different, destructive encoding) and vblendvps without VEX
        // is a guaranteed SIGILL.
        if self.isa.sse41 && self.avx2_enabled {
            let inst = match domain {
                BlendvDomain::Pd => "vblendvpd",
                _ => "vblendvps",
            };
            self.emit_blendv_128_vex(dest, args, inst);
            return;
        }
        let (and, andn, or) = match domain {
            BlendvDomain::Ps => ("andps", "andnps", "orps"),
            BlendvDomain::Pd => ("andpd", "andnpd", "orpd"),
            BlendvDomain::Int => ("pand", "pandn", "por"),
        };
        self.emit_sse_blendv_128_mnemonics(dest, args, and, andn, or);
    }

    /// Integer-domain lane-mask select (SSE2): pand/pandn/por — see
    /// `emit_sse_blendv_128`. The callers pick the width-aware VEX form
    /// first when the target and the LANE GRANULARITY admit it (dword
    /// lanes: vblendvps; word/byte lanes: vpblendvb — see the dispatch
    /// comments); this baseline stays the always-exact per-BIT select.
    pub(super) fn emit_sse_blendv_128_int(&mut self, dest: &Value, args: &[Operand]) {
        self.emit_sse_blendv_128_mnemonics(dest, args, "pand", "pandn", "por");
    }

    /// `vblendvX %mask, %true, %false, %dst` (128-bit, VEX — SSE4.1+AVX2
    /// gated, mnemonic picked by the CALLER for lane granularity):
    /// all-homed fast path with zero staging; otherwise the mask and
    /// false arm stream through the scratch pair with the true arm
    /// recovered from its home or slot (the audited blendv discipline,
    /// and the VEX /is4 slot constraints: the r/m operand is the AT&T
    /// SECOND source — the true arm — and VEX.vvvv the third — the false
    /// arm, register-only).
    fn emit_blendv_128_vex(&mut self, dest: &Value, args: &[Operand], inst: &str) {
        debug_assert!(args.len() == 3, "blendv128: expects false, true, mask");
        self.state.invalidate_vec_peephole();
        if matches!((&args[0], &args[1]), (Operand::Value(a), Operand::Value(b)) if a == b) {
            self.sse_load_arg(&args[0], "xmm0");
            self.sse_store_dest(dest, "xmm0");
            return;
        }
        // All-homed: one instruction, no staging.
        let f = self.vec_home_128(&args[0]);
        let t = self.vec_home_128(&args[1]);
        let m = self.vec_home_128(&args[2]);
        if let (Some(f), Some(t), Some(m)) = (&f, &t, &m) {
            if let Some(d) = self.dest_xmm_home_name(dest) {
                self.state
                    .emit_fmt(format_args!("    {} {}, {}, {}, %{}", inst, m, t, f, d));
                self.sse_commit_dest_direct(dest, d);
                return;
            }
        }
        // Generic: mask and FALSE arm through the scratch pair, true arm
        // recovered from home/slot — the VEX /is4 encoding constrains the
        // operand slots: AT&T `vblendvps %mask, %src2, %src1, %dst` puts
        // src2 in the ModRM.r/m (memory allowed) and src1 in VEX.vvvv
        // (REGISTER ONLY). lccc's blendv arg order is [false, true,
        // mask], so the FALSE arm is src1 (vvvv — must stream through a
        // register) and the TRUE arm is src2 (may take its slot as a
        // memory operand), exactly like `emit_avx_blendv_256`'s (None,
        // None) arm. The mask resolves first so a deferred cmp result
        // keeps its cache hit.
        self.sse_load_arg(&args[2], "xmm1");
        self.sse_load_arg(&args[0], "xmm0");
        let tsrc = match self.vec_home_128(&args[1]) {
            Some(reg) => reg,
            None => self.vec_mem_source_after_flush(&args[1], "blendv128"),
        };
        self.state
            .emit_fmt(format_args!("    {} %xmm1, {}, %xmm0, %xmm0", inst, tsrc));
        self.sse_store_dest(dest, "xmm0");
    }

    fn emit_sse_blendv_128_mnemonics(
        &mut self,
        dest: &Value,
        args: &[Operand],
        and: &str,
        andn: &str,
        or: &str,
    ) {
        // No leading flush: a deferred mask still held in %xmm0 must keep
        // its cache hit (the mask load below consumes it in place).  The
        // mask/true loads then commit or consume any other pending store,
        // so the false vector's memory source below is always fresh.
        self.state.invalidate_vec_peephole();
        if matches!((&args[0], &args[1]), (Operand::Value(a), Operand::Value(b)) if a == b) {
            self.sse_load_arg(&args[0], "xmm0");
            self.sse_store_dest(dest, "xmm0");
            return;
        }
        self.sse_load_arg(&args[2], "xmm1");
        self.sse_load_arg(&args[1], "xmm0");
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", and));
        let fsrc = match self.vec_home_128(&args[0]) {
            Some(reg) => reg,
            None => self.vec_mem_source_after_flush(&args[0], andn),
        };
        self.state
            .emit_fmt(format_args!("    {} {}, %xmm1", andn, fsrc));
        self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", or));
        self.sse_store_dest(dest, "xmm0");
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
            args.len() >= 5,
            "{} expects accumulator plus two base/offset pairs",
            mnemonic
        );
        // Optional trailing constant displacements (vec_interleave):
        // args[5] folds into the A-stream memory operand, args[6] into the
        // B-stream one (`vfmadd231pd 32(%rsi,%rax), %ymm1, %ymm5`).  Absent
        // for the plain 5-arg vectorizer form.
        let a_disp = Self::vec_disp_arg(args, 5);
        let b_disp = Self::vec_disp_arg(args, 6);

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

        let a_mem = self.vec_mem_operand(&args[1], &args[2], a_disp);
        self.state
            .emit_fmt(format_args!("    vmovdqu {}, %ymm0", a_mem));
        let b_mem = self.vec_mem_operand(&args[3], &args[4], b_disp);
        self.state.emit_fmt(format_args!(
            "    {} {}, %ymm0, %{}",
            mnemonic, b_mem, target
        ));

        self.state.vector_values.insert(dest.0);
        if let Some(reg) = assigned {
            let name = phys_reg_name_256(reg);
            self.state.vec_claim_live_reg(dest.0, name);
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
            .map(|pf| pf.val)
            .filter(|pv| {
                args.iter()
                    .any(|a| matches!(a, Operand::Value(v) if v.0 == *pv))
            });
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
        // never a per-iteration `vmovdqa %ymmS, %ymm1` copy. WIDTH-MATCHED:
        // only a 32-byte fold belongs to this 256-bit family.
        //
        // RESOLUTION IS MANDATORY: when a width-matched fold names one of
        // this op's operands (guaranteed by the emit_intrinsic_impl safety
        // net — a non-consuming op materialises it first), this branch must
        // either CONSUME the fold or MATERIALISE it. Falling through with
        // the fold still pending lets every later fast path read the
        // elided value's RA-reserved register, which the elided load never
        // wrote (the v6 clamp miscompile: the map-kernel path read %ymm3
        // directly). The register source is therefore resolved in THREE
        // ways, in order: the other operand's RA home, the scratch
        // register holding its DEFERRED store (the clamp shape: the zero
        // splat deferred in %ymm0, consumed in place), or — failing both —
        // a materialisation of the fold so the ordinary paths reload it.
        if let Some(pf) = self.state.pending_vec_memfold.clone() {
            if pf.width == 32 {
                let pv = pf.val;
                if let (Operand::Value(x), Operand::Value(y)) = (&args[0], &args[1]) {
                    let other = if y.0 == pv {
                        Some(x)
                    } else if x.0 == pv && commutative {
                        Some(y)
                    } else {
                        None
                    };
                    // The deferred-scratch register holding `other`, if its
                    // single-use store is still pending (256-bit values only:
                    // a 128-bit-named register would splice an illegal
                    // mixed-width operand into this YMM instruction).
                    let held: Option<&'static str> = other.and_then(|o| {
                        self.state
                            .pending_vec_store
                            .filter(|(p, _, wide)| *p == o.0 && *wide)
                            .map(|(_, r, _)| r)
                    });
                    let other_reg = other
                        .and_then(|o| self.reg_assignments.get(&o.0).copied())
                        .filter(|r| is_xmm_reg(*r));
                    let src: Option<String> = other_reg
                        .map(|oreg| phys_reg_name_256(oreg).to_string())
                        .or_else(|| held.map(|h| h.to_string()));
                    if let Some(src) = src {
                        // A deferred `other` is consumed here: its pending
                        // store never fires (pure win, exactly like the
                        // register-cache consumers).
                        if held.is_some() {
                            self.state.pending_vec_store = None;
                        }
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
                                avx_inst, pf.mem, src, dst
                            ));
                            self.state.vec_claim_live_reg(dest_ptr.0, dst);
                            self.state.vec_last_store_val = Some(dest_ptr.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(dst);
                            self.state.reg_cache.invalidate_acc();
                        } else {
                            // The destination takes the scratch: any OTHER
                            // def's deferred store in %ymm0 must be committed
                            // first (a held-`other` was already consumed
                            // above, so this flush can only be for a
                            // different value).
                            self.flush_pending_vec_store_impl();
                            self.state.emit_fmt(format_args!(
                                "    {} {}, %{}, %ymm0",
                                avx_inst, pf.mem, src
                            ));
                            self.state.vec_last_store_reg = false;
                            self.avx_store_dest(dest_ptr);
                        }
                        self.state.pending_vec_memfold = None;
                        return;
                    }
                    // No register source for the other operand (slot-homed
                    // splat, foreign-width... ): the fold cannot be consumed
                    // here. MATERIALISE it — never fall through with the
                    // elided value's reserved register still readable by the
                    // fast paths below.
                    if other.is_some() || x.0 == pv || y.0 == pv {
                        self.materialize_pending_memfold();
                    }
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
                        self.state.vec_claim_live_reg(dest_ptr.0, target);
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
                        self.state.vec_claim_live_reg(dest_ptr.0, nd);
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
                // A REGISTER-HOMED args[1] (the Adler epic's zero/weight
                // tables, map kernels' clamp bounds) is the VEX.vvvv
                // source DIRECTLY — the home is read-only here, so no
                // `%ymm1` staging copy is needed (`vpsadbw m, %ymm6, %ymm0`
                // instead of `vmovdqa %ymm6, %ymm1` + `vpsadbw m, %ymm1,
                // %ymm0`: one instruction off every iteration).
                if let Operand::Value(v1) = &args[1] {
                    if let Some(r1) = self
                        .reg_assignments
                        .get(&v1.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                    {
                        let n1 = phys_reg_name_256(r1);
                        self.state.dirty_upper_ymm = true;
                        self.state
                            .emit_fmt(format_args!("    {} {}, %{}, %ymm0", avx_inst, m0, n1));
                        self.state.vec_last_store_reg = false;
                        self.avx_store_dest(dest_ptr);
                        return;
                    }
                }
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

/// The closed 132→{213,231} family table (`fma_form_reencode`): every
/// mnemonic the map-FMA emitter can be dispatched with has both siblings,
/// and the re-encode preserves lane width and sign family exactly. A
/// mnemonic added to the dispatch tables without extending this match
/// fails the exhaustive test below (and the emitter's `unreachable!`).
#[cfg(test)]
mod fma_form_reencode_tests {
    use super::X86Codegen;

    #[test]
    fn every_family_mnemonic_has_both_siblings() {
        // (132, 213, 231) × {pd, ps} × {vfmadd, vfmsub, vfnmadd, vfnmsub}
        let families = ["vfmadd", "vfmsub", "vfnmadd", "vfnmsub"];
        let widths = ["pd", "ps"];
        for f in families {
            for w in widths {
                let m132 = format!("{f}132{w}");
                let m213 = format!("{f}213{w}");
                let m231 = format!("{f}231{w}");
                assert_eq!(
                    X86Codegen::fma_form_reencode(&m132, false),
                    m213,
                    "213 sibling of {m132}"
                );
                assert_eq!(
                    X86Codegen::fma_form_reencode(&m132, true),
                    m231,
                    "231 sibling of {m132}"
                );
            }
        }
    }

    #[test]
    fn non_132_inputs_are_rejected_not_passthrough() {
        // The whole point of the closed table: a mnemonic that is not a
        // 132-family spelling must fail loudly (in tests) rather than
        // silently round-trip (the `str::replace` behaviour). Both the
        // already-re-encoded forms and foreign mnemonics are rejected.
        let bad = [
            "vfmadd213pd",
            "vfmadd231ps",
            "vpaddd",
            "vmovupd",
            "vfmadd132sd",
            "",
        ];
        for m in bad {
            let r213 = std::panic::catch_unwind(|| X86Codegen::fma_form_reencode(m, false));
            let r231 = std::panic::catch_unwind(|| X86Codegen::fma_form_reencode(m, true));
            assert!(r213.is_err(), "{m} must not re-encode to 213");
            assert!(r231.is_err(), "{m} must not re-encode to 231");
        }
    }
}
