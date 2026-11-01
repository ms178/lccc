//! X86Codegen: function call operations.

use super::emit::{X86_ARG_REGS, X86Codegen};
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, compute_stack_push_bytes};
use crate::backend::generation::is_i128_type;
use crate::backend::narrow_copy::{NarrowCopy, eax_ferry, narrow_copy_plan, narrow_move_mnemonic};
use crate::backend::state::{SibcallPending, SibcallTarget};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{CallInfo, Instruction, IrBinOp, IrConst, Operand, Value};

/// How a resolved global call target must be emitted.
pub(super) enum ResolvedIndirectCall {
    /// func_ptr was `Load(ptr)` with ptr = GlobalAddr(sym)+off: the loaded
    /// VALUE is the target, so fold load+call into the patchable
    /// memory-indirect form `call *sym+off(%rip)` (ff 15).
    MemorySlot(String, i64),
    /// func_ptr's VALUE itself resolves to sym+off (a GlobalAddr chain with
    /// no intervening Load): the value IS the target, so it must be a
    /// DIRECT `call sym+off`. Emitting `call *sym(%rip)` here would read
    /// sym's first 8 code/data bytes as the target (pic_extern_addr
    /// SIGSEGV: `call *add7(%rip)` jumped into `movq %rdi+7, %rax`).
    Direct(String, i64),
}

/// 32-bit name of a 64-bit general register (`rcx` -> `ecx`, `r8` -> `r8d`).
fn dword_name(qword: &str) -> String {
    match qword.strip_prefix('r') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => format!("r{rest}d"),
        Some(rest) => format!("e{rest}"),
        None => qword.to_string(),
    }
}

impl X86Codegen {
    /// Load the bytes `[eb, struct_size)` of a by-value struct argument into
    /// the 64-bit register `dst`, zero-extended, reading exactly those bytes.
    ///
    /// `%rax` holds the struct's address. `eb` is the eightbyte's byte offset (0
    /// or 8), and the eightbyte has `r = min(8, struct_size - eb)` bytes:
    ///
    /// * `r == 8`: one qword load.
    /// * `eb >= 8` (a partial high eightbyte, `struct_size >= 9`): load the
    ///   window `[struct_size - 8, struct_size)`, which ends at the object, and
    ///   shift out its leading `8 - r` bytes. No byte before the object is read
    ///   and no scratch register is needed.
    /// * `eb == 0` and `r < 8`: the struct is smaller than one qword. Compose
    ///   it from two overlapping narrow loads (see `load_bytes_exact` in the
    ///   IR lowering); `%rax` is free afterwards because nothing else reads it.
    ///
    /// The previous emission loaded a full qword in all three cases, which read
    /// up to 7 bytes past the end of the struct.
    /// Load eightbyte `eb` of a by-value struct whose address is in `%rax` into
    /// `dst`, reading only the bytes that belong to the struct. Full qwords keep
    /// the `(%rax)` form of the whole-qword load they replace, so the encoding is
    /// unchanged for the common case. A partial width uses an exact-width load
    /// (1, 2, 4) or two overlapping loads (3, 5, 6, 7). The 3/5/6/7 form clobbers
    /// `%rax`: it is only emitted for a single-eightbyte struct, where `%rax` is
    /// not needed afterwards.
    pub(super) fn emit_struct_eightbyte_exact(&mut self, struct_size: usize, eb: usize, dst: &str) {
        // Bytes of this eightbyte that belong to the struct: a full qword for
        // any eightbyte before the last one, the remainder for the last one.
        let r = (struct_size - eb).min(8);
        debug_assert!((1..=8).contains(&r), "eightbyte width {r}");
        let mem = |off: usize| {
            if off == 0 {
                "(%rax)".to_string()
            } else {
                format!("{off}(%rax)")
            }
        };
        if r == 8 {
            self.state
                .emit_fmt(format_args!("    movq {}, %{}", mem(eb), dst));
            return;
        }
        if eb >= 8 {
            self.state
                .emit_fmt(format_args!("    movq {}, %{}", mem(struct_size - 8), dst));
            self.state
                .emit_fmt(format_args!("    shrq ${}, %{}", 8 * (8 - r), dst));
            return;
        }
        debug_assert_eq!(eb, 0);
        let d32 = dword_name(dst);
        match r {
            1 => self
                .state
                .emit_fmt(format_args!("    movzbl {}, %{d32}", mem(0))),
            2 => self
                .state
                .emit_fmt(format_args!("    movzwl {}, %{d32}", mem(0))),
            4 => self
                .state
                .emit_fmt(format_args!("    movl {}, %{d32}", mem(0))),
            _ => {
                // 3, 5, 6, 7: overlapping low and high loads of width w.
                let w = if r == 3 { 2 } else { 4 };
                if w == 2 {
                    self.state
                        .emit_fmt(format_args!("    movzwl {}, %{d32}", mem(0)));
                    self.state
                        .emit_fmt(format_args!("    movzwl {}, %eax", mem(r - w)));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{d32}", mem(0)));
                    self.state
                        .emit_fmt(format_args!("    movl {}, %eax", mem(r - w)));
                }
                self.state
                    .emit_fmt(format_args!("    shlq ${}, %rax", 8 * (r - w)));
                self.state.emit_fmt(format_args!("    orq %rax, %{dst}"));
            }
        }
    }

    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig::sysv_x86_64()
    }

    /// Extract a constant i64 from an Operand, following Copy chains.
    fn operand_const_i64(&self, op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(c) => c.to_i64(),
            Operand::Value(v) => {
                let mut cur = v.0;
                let mut visited = 0;
                loop {
                    if visited > 16 {
                        return None;
                    }
                    visited += 1;
                    let inst = self.get_defining_instruction(cur)?;
                    match inst {
                        Instruction::Copy { src, .. } => match src {
                            Operand::Const(c) => return c.to_i64(),
                            Operand::Value(nv) => cur = nv.0,
                        },
                        Instruction::Cast { src, .. } => match src {
                            Operand::Const(c) => return c.to_i64(),
                            Operand::Value(nv) => cur = nv.0,
                        },
                        _ => return None,
                    }
                }
            }
        }
    }

    /// Resolve a Value chain that ends in GlobalAddr, accumulating GEP/Add/Sub/Cast offsets.
    /// Returns (symbol_name, total_offset) if the chain is GlobalAddr (+ GEPs/Copies/Adds/Subs/Casts).
    /// This is the paravirt fast-path: `pv_ops` is a struct of function pointers, so
    /// `Load(GEP(GlobalAddr pv_ops, off))` must become `call *pv_ops+off(%rip)` (ff 15)
    /// for `apply_alternatives` to patch. The resolver is intentionally generous:
    /// it follows Cast (bitcast/ptrtoint/inttoptr), Copy, Add (Value+Const), Sub (Value-Const),
    /// and GEP with const or Copy-of-const offsets, with checked arithmetic and 64-hop bound.
    /// The symbol name is returned VERBATIM (version suffix intact) — folding policy
    /// (`.symver` rejection, GOT/TLS/absolute guards) lives in the caller.
    fn resolve_global_addr_chain(&self, mut val_id: u32, mut acc: i64) -> Option<(String, i64)> {
        let mut visited = 0;
        loop {
            if visited > 64 {
                return None;
            }
            visited += 1;
            let inst = self.get_defining_instruction(val_id)?;
            match inst {
                Instruction::GlobalAddr { name, .. } => {
                    return Some((name.clone(), acc));
                }
                Instruction::GetElementPtr { base, offset, .. } => {
                    let off = self.operand_const_i64(offset)?;
                    acc = acc.checked_add(off)?;
                    val_id = base.0;
                }
                Instruction::Copy { src, .. } => match src {
                    Operand::Value(v) => val_id = v.0,
                    Operand::Const(_) => return None, // Copy of const is not a global chain
                },
                Instruction::Cast { src, .. } => match src {
                    Operand::Value(v) => val_id = v.0,
                    Operand::Const(_) => return None,
                },
                Instruction::BinOp { op, lhs, rhs, .. } => match op {
                    IrBinOp::Add => {
                        let (base_val, const_off) = match (lhs, rhs) {
                            (Operand::Value(v), c) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            (c, Operand::Value(v)) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            _ => return None,
                        };
                        acc = acc.checked_add(const_off)?;
                        val_id = base_val;
                    }
                    IrBinOp::Sub => {
                        // Only Value - Const is representable as sym+off (off decreases)
                        let (base_val, const_off) = match (lhs, rhs) {
                            (Operand::Value(v), c) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            _ => return None,
                        };
                        acc = acc.checked_sub(const_off)?;
                        val_id = base_val;
                    }
                    _ => return None,
                },
                _ => return None,
            }
        }
    }

    /// Try to resolve an indirect call's func_ptr Operand to a global+offset.
    /// The func_ptr is expected to be Value(Load(ptr)), where ptr is GEP chain
    /// ending in GlobalAddr (e.g. pv_ops+72). Returns Some((sym, off)) if so.
    /// Also handles the case where Load was folded or func_ptr itself is the GEP chain.
    ///
    /// Versioned symbols (`sym@VER`, bound by a top-level `.symver real, sym@VER`
    /// directive) are rejected outright: the memory-indirect form
    /// `call *sym@VER+off(%rip)` is rejected by GAS 2.47, the stripped base
    /// name would silently re-bind the call to the DEFAULT version (a
    /// .symver semantic change, not a fold), and the GOT/TLS/absolute guards
    /// below are keyed by the full symbol name. The value-based fallback
    /// (load the pointer, call through it) is correct for every binding, so
    /// rejecting the fold is strictly safe — and versioned function-pointer
    /// slots are glibc-internal compat shapes, so the cost is nil.
    pub(super) fn try_resolve_indirect_call_global(
        &self,
        op: &Operand,
    ) -> Option<ResolvedIndirectCall> {
        let v = match op {
            Operand::Value(v) => v,
            _ => return None,
        };
        let inst = self.get_defining_instruction(v.0)?;
        match inst {
            Instruction::Load { ptr, .. } => {
                // If the loaded pointer's defining chain is global+off, we can fold into
                // the memory-indirect form. Guard against GOT/TLS/absolute symbols that
                // need different addressing.
                let (sym, off) = self.resolve_global_addr_chain(ptr.0, 0)?;
                if sym.contains('@') {
                    return None;
                }
                // Don't use RIP-relative call for symbols that need GOT or are TLS/absolute —
                // those would need GOTPCREL or fs: addressing, not plain *sym(%rip).
                if self.state.needs_got_for_addr(&sym)
                    || self.state.tls_symbols.contains(&sym)
                    || self.state.absolute_symbols.contains(&sym)
                {
                    return None;
                }
                Some(ResolvedIndirectCall::MemorySlot(sym, off))
            }
            // Defensive: func_ptr itself might be the GlobalAddr/GEP chain (if the
            // Load was optimized away). The resolved symbol+offset is then the call
            // target value itself, so only a DIRECT call is semantically correct.
            _ => {
                let (sym, off) = self.resolve_global_addr_chain(v.0, 0)?;
                if sym.contains('@') {
                    return None;
                }
                if self.state.needs_got_for_addr(&sym)
                    || self.state.tls_symbols.contains(&sym)
                    || self.state.absolute_symbols.contains(&sym)
                {
                    return None;
                }
                Some(ResolvedIndirectCall::Direct(sym, off))
            }
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        struct_arg_aligns: &[Option<usize>],
        _struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],
        _struct_arg_is_f128_sse: &[bool],
    ) -> usize {
        // x86-64: uncapped per-arg alignment padding (GCC ≥ 4.6 honors the
        // full natural alignment of MEMORY-class stack arguments; the >16
        // case is completed by the dynamic %rsp realignment in
        // emit_call_stack_args_impl).
        compute_stack_push_bytes(arg_classes, struct_arg_aligns, usize::MAX)
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
        struct_arg_aligns: &[Option<usize>],
        _struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],

        struct_arg_is_f128_sse: &[bool],
    ) -> i64 {
        // Per-call-site transient state (defensive reset: the pipeline
        // always consumes the flag, but a stale value from an aborted
        // emission path must never leak into the next call).
        self.dyn_align_cleanup = false;

        // SysV AMD64 (GCC ≥ 4.6 ABI change, flagged by GCC as "the ABI for
        // passing parameters with N-byte alignment has changed in GCC 4.6"):
        // a MEMORY-class argument whose natural alignment exceeds 16 bytes is
        // placed at a slot aligned to its FULL alignment. The callee's va_arg
        // overflow walk aligns dynamically (addq $A-1; andq $-A — see
        // emit_va_arg_struct_overflow_body), anchored at the FIRST stack
        // argument's address (= (%rsp) at the call, where the callee finds it
        // at entry_rsp+8). A purely static layout cannot guarantee that
        // anchor ≡ 0 mod A — the pre-push %rsp is only known to be 16-aligned,
        // so %rsp mod 32/64 is runtime-dependent. When any stack argument
        // needs alignment A > 16, the outgoing area is built GCC-style:
        //   (1) %rsp is realigned BEFORE any push, by D = (R0 − TOTAL) mod A
        //       where TOTAL = save slot + parity pad + argument area is the
        //       static byte count the pushes will consume — so the FINAL
        //       (%rsp) at the call (the anchor) lands exactly on an A
        //       boundary. A is a multiple of 16 and TOTAL is 16-rounded, so D
        //       is a multiple of 16 and %rsp stays 16-aligned at the call;
        //   (2) a 16-byte slot above every argument (the callee's overflow
        //       walk never reads past the last argument it consumes) saves
        //       the pre-realignment %rsp for the exact cleanup restore —
        //       correct even when a DynAlloca moved %rsp between prologue and
        //       this call;
        //   (3) the cleanup skips the static argument area and restores %rsp
        //       exactly from the save slot.
        // The dynamic %rsp delta forbids %rsp-relative slot addressing, so
        // generation.rs pins such functions to an rbp frame (exactly GCC's
        // answer: it realigns the whole frame). %rcx/%rax are free here —
        // register arguments are loaded in Phase 3, after this function.
        let dyn_align_a =
            crate::backend::call_abi::max_stack_arg_alignment(arg_classes, struct_arg_aligns);
        // generation.rs pins functions with >16-aligned stack args to an rbp
        // frame (the dynamic %rsp delta forbids %rsp-relative addressing),
        // so the realign path cannot be reached in FPO mode; the fallback
        // keeps the emission self-consistent if that invariant is ever broken.
        debug_assert!(dyn_align_a <= 16 || !self.state.out.use_rsp_addressing);
        let dyn_align = dyn_align_a > 16 && !self.state.out.use_rsp_addressing;

        let mut sp_adjust: i64 = 0;
        if dyn_align {
            let area = stack_arg_space as i64;
            let parity_pad = if stack_arg_space % 16 != 0 { 8 } else { 0 };
            let total = 16 + parity_pad + area; // 16-rounded by construction
            let a = dyn_align_a as i64;
            // (1) realign: %rsp -= (R0 - TOTAL) mod A  → anchor ≡ 0 (mod A)
            self.state.emit("    movq %rsp, %rcx"); // %rcx = R0 (scratch only)
            self.state.emit("    movq %rcx, %rax");
            self.state
                .emit_fmt(format_args!("    subq ${}, %rax", total));
            self.state
                .emit_fmt(format_args!("    andq ${}, %rax", a - 1));
            self.state.emit("    subq %rax, %rsp");
            // (2) save the pre-realignment %rsp above every argument
            self.state.emit("    subq $16, %rsp");
            self.state.emit("    movq %rcx, (%rsp)");
            sp_adjust += 16;
            if parity_pad > 0 {
                self.state
                    .out
                    .emit_instr_imm_reg("    subq", parity_pad, "rsp");
                sp_adjust += parity_pad;
            }
            // S20 F-CC11: the realignment scratch above overwrites BOTH cached
            // registers (%rcx = R0, %rax = the modulo chain), and Phase 2 has
            // no exit invalidate — the per-argument fix (previous_arg_wrote_rcx
            // below) only covers writes INSIDE the Phase-3 register loop, so
            // without this a SEC/ACC park from before the call would outlive
            // the scratch and misfire on the first Phase-3 SEC/ACC consume.
            // Narrow in practice (over-aligned dynamic realignment), but the
            // contract is unconditional: every cached-register write maintains
            // the caches. Placed inside the arm so non-realigned calls pay
            // nothing — not even the metadata store.
            self.state.reg_cache.invalidate_all();
            self.dyn_align_cleanup = true;
        } else if stack_arg_space % 16 != 0 {
            // Parity pad below the outgoing area. `pushq $0` (6A 00, 2 B)
            // replaces `subq $8, %rsp` (48 83 EC 08, 4 B): identical %rsp
            // delta, and the dead qword it defines is never read — it sits
            // BELOW every argument the callee walks, so the argument view
            // is unchanged. One instruction fewer, two bytes fewer, and no
            // dependency on the (possibly busy) %rax staging lane.
            if self.state.ra_config.no_x64_push_arg_forms {
                self.state.emit("    subq $8, %rsp");
            } else {
                self.state.emit("    pushq $0");
            }
            sp_adjust += 8;
            // Adjust RSP frame size so operand_to_rax slot conversions
            // are correct after the alignment subq.
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 8;
            }
        }
        let arg_padding = crate::backend::call_abi::compute_stack_arg_padding(
            arg_classes,
            struct_arg_aligns,
            usize::MAX,
        );
        let stack_indices: Vec<usize> = (0..args.len())
            .filter(|&i| arg_classes[i].is_stack())
            .collect();
        for &si in stack_indices.iter().rev() {
            match arg_classes[si] {
                CallArgClass::F128Stack => {
                    match &args[si] {
                        Operand::Const(c) => {
                            let x87_bytes: [u8; 10] = match c {
                                IrConst::LongDouble(_, f128_bytes) => {
                                    let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(
                                        f128_bytes,
                                    );
                                    let mut b = [0u8; 10];
                                    b.copy_from_slice(&x87[..10]);
                                    b
                                }
                                _ => {
                                    let f64_val = c.to_f64().unwrap_or(0.0);
                                    crate::ir::reexports::f64_to_x87_bytes(f64_val)
                                }
                            };
                            // SysV x86-64 passes long double in a 16-byte stack
                            // slot (the low 10 bytes are x87 extended precision).
                            // Account for the reservation before emitting any
                            // RSP-relative operand, otherwise `slot_ref`/`fldt`
                            // address the pre-subtraction stack location.
                            self.state.emit("    subq $16, %rsp");
                            sp_adjust += 16;
                            if self.state.out.use_rsp_addressing {
                                self.state.out.rsp_frame_size += 16;
                            }
                            let lo = u64::from_le_bytes(x87_bytes[0..8].try_into().unwrap());
                            let hi_2bytes =
                                u16::from_le_bytes(x87_bytes[8..10].try_into().unwrap());
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state
                                .emit_fmt(format_args!("    movw ${}, 8(%rsp)", hi_2bytes));
                            self.state.reg_cache.invalidate_all();
                            self.flush_pending_vec_store_impl();
                            self.state.invalidate_vec_peephole();
                        }
                        Operand::Value(v) => {
                            if self.state.f128_direct_slots.contains(&v.0) {
                                self.state.emit("    subq $16, %rsp");
                                sp_adjust += 16;
                                if self.state.out.use_rsp_addressing {
                                    self.state.out.rsp_frame_size += 16;
                                }
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                    self.state.emit("    fstpt (%rsp)");
                                }
                            } else if let Some(slot) = self.state.get_slot(v.0) {
                                if self.state.is_alloca(v.0) {
                                    self.state.emit("    subq $16, %rsp");
                                    sp_adjust += 16;
                                    if self.state.out.use_rsp_addressing {
                                        self.state.out.rsp_frame_size += 16;
                                    }
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                    self.state.emit("    fstpt (%rsp)");
                                } else {
                                    // The value is represented as a scalar f64
                                    // bit-pattern. Read it before reserving the
                                    // outgoing slot, then widen in that slot;
                                    // this avoids an untracked push/pop window.
                                    self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
                                    self.state.emit("    subq $16, %rsp");
                                    sp_adjust += 16;
                                    if self.state.out.use_rsp_addressing {
                                        self.state.out.rsp_frame_size += 16;
                                    }
                                    self.state.emit("    movq %rax, (%rsp)");
                                    self.state.emit("    fldl (%rsp)");
                                    self.state.emit("    fstpt (%rsp)");
                                }
                            } else {
                                self.state.emit("    subq $16, %rsp");
                                sp_adjust += 16;
                                if self.state.out.use_rsp_addressing {
                                    self.state.out.rsp_frame_size += 16;
                                }
                            }
                            self.state.reg_cache.invalidate_all();
                            self.flush_pending_vec_store_impl();
                            self.state.invalidate_vec_peephole();
                        }
                    }
                }
                CallArgClass::I128Stack => {
                    // Read both halves before changing RSP.  The old code
                    // advanced rsp_frame_size before these slot_ref calls, so
                    // a source i128 was read from the outgoing argument area
                    // and its halves were swapped/corrupted.
                    match &args[si] {
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Push both halves straight from the slot.  No
                                // scratch register may be used here: register
                                // arguments are staged AFTER the stack
                                // arguments, and the allocator may keep one of
                                // them in %r11 -- `c(.., p0 ^ 3, .., i128)`
                                // with every GP slot taken pushed the high
                                // limb through %r11 and passed it as `p0 ^ 3`
                                // (call-arg staging gate, five seeds of forty).
                                // Under %rsp addressing the first push moves
                                // the slot 8 bytes further away.
                                self.state.out.emit_instr_rbp("    pushq", slot.0 + 8);
                                let rsp_rel = self.state.out.use_rsp_addressing;
                                if rsp_rel {
                                    self.state.out.rsp_frame_size += 8;
                                }
                                self.state.out.emit_instr_rbp("    pushq", slot.0);
                                if rsp_rel {
                                    self.state.out.rsp_frame_size -= 8;
                                }
                            } else {
                                self.state.emit("    pushq $0");
                                self.state.emit("    pushq $0");
                            }
                        }
                        Operand::Const(c) => {
                            if let IrConst::I128(v) = c {
                                // High limb first.  `pushq $imm` takes a
                                // sign-extended imm32 only; a wider limb goes
                                // through %rax, the stack-argument scratch the
                                // scalar path uses too (never allocated).
                                for limb in [(*v >> 64) as i64, *v as i64] {
                                    if i32::try_from(limb).is_ok() {
                                        self.state.emit_fmt(format_args!("    pushq ${limb}"));
                                    } else {
                                        self.state.out.emit_instr_imm_reg(
                                            "    movabsq",
                                            limb,
                                            "rax",
                                        );
                                        self.state.emit("    pushq %rax");
                                    }
                                }
                                self.state.reg_cache.invalidate_all();
                            } else {
                                // Smaller constant or zero
                                if let Operand::Value(_) = &args[si] {} // can't happen
                                self.operand_to_rax(&args[si]);
                                self.state.emit("    pushq $0");
                                self.state.emit("    pushq %rax");
                            }
                        }
                    }
                    // Subsequent stack arguments and register arguments see
                    // the two pushed qwords at their new RSP-relative offsets.
                    sp_adjust += 16;
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 16;
                    }
                }
                CallArgClass::StructByValStack { size: 16 }
                    if crate::backend::call_abi::is_scalar_f128_sse_arg(
                        arg_types,
                        struct_arg_is_f128_sse,
                        si,
                    ) =>
                {
                    // Scalar _Float128/_Decimal128 overflow keeps the same
                    // 16-byte stack slot as a one-XMM aggregate, but its
                    // operand is the VALUE, not an aggregate address. Using
                    // operand_to_rax here truncated the I128 image to its low
                    // limb and dereferenced it (binary128 9.0 -> NULL).
                    self.state.emit("    subq $16, %rsp");
                    sp_adjust += 16;
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 16;
                    }
                    self.emit_f128_sse_stack_arg(&args[si]);
                }
                CallArgClass::StructByValStack { size }
                | CallArgClass::LargeStructStack { size } => {
                    self.operand_to_rax(&args[si]);
                    let n_qwords = size.div_ceil(8);
                    if size >= crate::backend::call_abi::LARGE_ARG_COPY_THRESHOLD {
                        let push_bytes = (n_qwords * 8) as i64;
                        self.state
                            .out
                            .emit_instr_imm_reg("    subq", push_bytes, "rsp");
                        // Register arguments are staged AFTER stack arguments.
                        // Preserve their possible homes, and leave the temporary
                        // save area BELOW the ABI argument so popping it does
                        // not introduce a hole between adjacent arguments.
                        self.state.emit("    pushq %rdi");
                        self.state.emit("    pushq %rsi");
                        self.state.emit("    pushq %rcx");
                        self.state.emit("    movq %rax, %rsi");
                        self.state.emit("    leaq 24(%rsp), %rdi");
                        self.state
                            .out
                            .emit_instr_imm_reg("    movq", size as i64, "rcx");
                        self.state.emit("    rep movsb");
                        self.state.emit("    popq %rcx");
                        self.state.emit("    popq %rsi");
                        self.state.emit("    popq %rdi");
                    } else {
                        for qi in (0..n_qwords).rev() {
                            let offset = qi * 8;
                            if offset + 8 <= size {
                                self.state
                                    .emit_fmt(format_args!("    pushq {}(%rax)", offset));
                            } else {
                                // Partial tail: exact bytes only, through the helper that
                                // also serves the register-argument path (no over-read).
                                self.emit_struct_eightbyte_exact(size, offset, "rcx");
                                self.state.emit("    pushq %rcx");
                            }
                        }
                    }
                    let push_bytes = (n_qwords * 8) as i64;
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += push_bytes;
                    }
                    sp_adjust += push_bytes;
                }
                CallArgClass::Stack => {
                    // Direct push forms first: `pushq $imm` / `pushq %reg` /
                    // `pushq slot(%rsp)` are each strictly smaller than the
                    // materialise-into-%rax fallback (6-9 B -> 1-8 B) and
                    // keep %rax out of the staging chain, so consecutive
                    // pushes no longer serialise through the accumulator.
                    // `try_emit_direct_stack_arg_push` is freshness-gated
                    // end to end and returns false whenever no proven form
                    // applies, leaving the classic path as the only writer
                    // of %rax.
                    let direct = !self.state.ra_config.no_x64_push_arg_forms
                        && self.try_emit_direct_stack_arg_push(&args[si]);
                    if !direct {
                        self.operand_to_rax(&args[si]);
                        self.state.emit("    pushq %rax");
                    }
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 8;
                    }
                    sp_adjust += 8;
                }
                _ => {}
            }
            let pad = arg_padding[si];
            if pad > 0 {
                self.state
                    .out
                    .emit_instr_imm_reg("    subq", pad as i64, "rsp");
                sp_adjust += pad as i64;
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size += pad as i64;
                }
            }
        }
        // Restore the original RSP frame size. The total adjustment is
        // tracked in sp_adjust and returned to emit_call_reg_args. (The
        // dynamic realignment delta is NOT tracked — it is invisible to the
        // rbp-relative addressing that such functions are pinned to.)
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= sp_adjust;
        }
        // Return the total STATIC RSP adjustment so emit_call_reg_args can
        // compensate stack slot offsets when loading register arguments.
        sp_adjust
    }

    /// Emit one DIRECT push for a MEMORY-class scalar stack argument.
    ///
    /// Three forms, in descending win size — each must be provably equal to
    /// `operand_to_rax` + `pushq %rax` at the moment of the push, or the
    /// method returns false and the caller takes the fallback:
    ///
    /// 1. `pushq $imm` (2/5 B) — [`x64_stack_arg_push_imm`] owns the
    ///    sign-extension law (imm8 window, narrow-lane ABI latitude,
    ///    eight-byte fit test);
    /// 2. `pushq %reg` (1/2 B) — the value's CURRENT home, freshness-gated
    ///    through `fresh_home_of` (a stale home would push a derived
    ///    value); %rsp is never a home, and %rbp is only pushed in FPO
    ///    mode (with an rbp frame it is the frame pointer);
    /// 3. `pushq slot(%rsp|%rbp)` (5/8 B) — the value's slot, resolved only
    ///    when it has NO register home at all (`SlotAddr::Direct`; indirect
    ///    slot addressing falls back), so the displacement is the plain
    ///    `slot_ref` of the live frame bookkeeping — the same offset
    ///    `operand_to_rax`'s load would use, because the frame accounting
    ///    has already absorbed every earlier push of this loop.
    ///
    /// The accumulator cache is intentionally NOT invalidated: no form
    /// writes %rax, so a live cached value stays cached (the fallback path
    /// overwrites it through `operand_to_rax` as before).
    fn try_emit_direct_stack_arg_push(&mut self, op: &Operand) -> bool {
        match op {
            Operand::Const(c) => {
                if let Some(imm) = super::emit::x64_stack_arg_push_imm(c) {
                    self.state.emit_fmt(format_args!("    pushq ${}", imm));
                    return true;
                }
                false
            }
            Operand::Value(v) => {
                // (1) live accumulator: `pushq %rax` is exactly the
                // fallback's push, minus the materialisation.
                let is_alloca = self.state.is_alloca(v.0);
                if self.state.acc_has_verified(v.0, is_alloca) {
                    self.state.emit("    pushq %rax");
                    return true;
                }
                // (2) secondary cache (%rcx): same law.
                if self.state.sec_has_verified(v.0, is_alloca) {
                    self.state.emit("    pushq %rcx");
                    return true;
                }
                // (3) fresh register home — GP only; an XMM home has no
                // push form and must take the fallback.
                if let Some(reg) = self.fresh_home_of(v.0) {
                    if super::emit::is_gpr_reg(reg)
                        && (reg.0 != 6 || self.state.out.use_rsp_addressing)
                    {
                        let name = super::emit::phys_reg_name(reg);
                        self.state.emit_fmt(format_args!("    pushq %{}", name));
                        return true;
                    }
                    // XMM homes fall through to the fallback.
                    return false;
                }
                // (4) plain scalar slot holding the full value: only when
                // no register home exists at all (a stale home would need a
                // reload first — but the slot is then the authoritative
                // copy, which is exactly what the fallback loads), the
                // slot resolves DIRECTLY, and the value is none of the
                // shapes for which the slot is NOT the scalar itself: an
                // alloca (slot holds locals, the value is its address),
                // i128/F128/vector payloads (8 bytes are not the value), or
                // a small slot (its 4 bytes back a narrower datum; the
                // fallback's load re-establishes the type invariant, a
                // blind 8-byte push would not).
                // (4) plain scalar slot holding the full value.  Reached
                // only with no fresh home: either no register home exists,
                // or the home is stale — in both cases the slot is the
                // authoritative copy, which is exactly what the fallback
                // (operand_to_rax's slot path) loads, so pushing from it is
                // the same data.  NOTE: `get_slot` (the value's storage
                // slot), not `resolve_slot_addr` — that API classifies
                // plain scalars as Indirect because it answers a different
                // question (the address OF a value for lea/memcpy shapes).
                // Refused when the slot is NOT the scalar itself: an alloca
                // (slot holds locals, the value is its address),
                // i128/F128/vector payloads (8 bytes are not the value), or
                // a small slot (its 4 bytes back a narrower datum; the
                // fallback's load re-establishes the type invariant, a
                // blind 8-byte push would not).
                let plain_scalar_slot = !is_alloca
                    && !self.state.i128_values.contains(&v.0)
                    && !self.state.f128_direct_slots.contains(&v.0)
                    && !self.state.vector_values.contains(&v.0)
                    && !self.state.vector128_values.contains(&v.0)
                    && !self.state.is_small_slot(v.0);
                if plain_scalar_slot {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let sr = self.slot_ref(slot.0);
                        self.state.emit_fmt(format_args!("    pushq {}", sr));
                        return true;
                    }
                }
                false
            } // Operand is the two-variant (Const | Value) IR operand enum;
              // the match above is exhaustive on purpose.  A new variant must
              // decide its own push story here instead of silently falling
              // through a wildcard.
        }
    }

    /// Spill an indirect function pointer before stack argument setup.
    ///
    /// The shared call pipeline pushes stack arguments before emitting the call.
    /// If we reload the function pointer from an RSP-addressed stack slot after
    /// those pushes, the slot offset is wrong and the indirect call may jump to
    /// NULL.  Keep the target in r10 instead: r10 is not an ABI argument register
    /// on SysV x86-64, and caller-saved live values have already been spilled by
    /// `emit_pre_call_save_caller_regs` before this hook runs.
    pub(super) fn emit_call_spill_fptr_impl(&mut self, func_ptr: &Operand) {
        self.indirect_fptr_on_stack = false;
        // Statically resolved targets (a paravirt data cell, or a direct
        // `&symbol` value) need no r10 spill: emit_call_instruction_impl
        // emits the resolved call form directly.
        if self.try_resolve_indirect_call_global(func_ptr).is_some() {
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        self.operand_to_rax(func_ptr);
        self.state.emit("    movq %rax, %r10");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// Defense-in-depth wrapper for indirect-call target pre-staging: RA's
    /// `indirect_target_regs` exclusion keeps indirect-call arguments off
    /// `%r10` (`PhysReg(11)`), so the zero-stack `%r10` fast path fires on
    /// every normal compilation. If an argument ever arrives homed in `%r10`
    /// (e.g. under a bisect flag or future RA pass), park the callee pointer
    /// in a 16-byte transient stack slot until stack/register argument staging
    /// has finished reading `%r10`, then reload `%r10` right before the call.
    pub(super) fn emit_call_spill_fptr_with_args_impl(
        &mut self,
        func_ptr: &Operand,
        args: &[Operand],
    ) {
        self.indirect_fptr_on_stack = false;
        let arg_uses_r10 = args.iter().any(|op| {
            matches!(
                op,
                Operand::Value(v)
                    if self
                        .reg_assignments
                        .get(&v.0)
                        .is_some_and(|phys| phys.0 == 11)
            )
        });
        if !arg_uses_r10 {
            self.emit_call_spill_fptr_impl(func_ptr);
            return;
        }
        if self.try_resolve_indirect_call_global(func_ptr).is_some() {
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        self.operand_to_rax(func_ptr);
        self.state.emit("    subq $16, %rsp");
        self.state.emit("    movq %rax, (%rsp)");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += 16;
        }
        self.indirect_fptr_on_stack = true;
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    fn reload_spilled_indirect_fptr_to_r10(&mut self, stack_arg_space: usize) {
        if !self.indirect_fptr_on_stack {
            return;
        }
        if self.dyn_align_cleanup {
            // Regular call only: the dyn-align save slot holds the pre-
            // realignment %rsp, whose top qword is the parked pointer.
            // emit_call_cleanup_impl owns the slot reclaim on this path; an
            // armed sibling call can never be dynamically realigned (its
            // zero-stack-argument guard rules the whole realignment arm out).
            debug_assert!(stack_arg_space > 0);
            let area =
                ((stack_arg_space + 15) & !15) + if self.state.call_shadow_active { 32 } else { 0 };
            self.state
                .out
                .emit_instr_mem_reg("    movq", area as i64, "rsp", "r10");
            self.state.emit("    movq (%r10), %r10");
            return;
        }
        let need_align_pad = stack_arg_space % 16 != 0;
        let pushed_bytes = stack_arg_space
            + if need_align_pad { 8 } else { 0 }
            + if self.state.call_shadow_active { 32 } else { 0 };
        if pushed_bytes == 0 {
            // No outgoing stack area (this includes every armed sibling call):
            // the parked slot is the top of %rsp, so reload and reclaim it
            // immediately; the flag must be clear before the next emission.
            self.state.emit("    movq (%rsp), %r10");
            self.state.emit("    addq $16, %rsp");
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= 16;
            }
            self.indirect_fptr_on_stack = false;
        } else {
            // Regular call with stack args: the slot sits ABOVE the pushed
            // outgoing area at %rsp + pushed_bytes. It stays allocated across
            // the call; emit_call_cleanup_impl reclaims it via fptr_extra.
            self.state
                .out
                .emit_instr_mem_reg("    movq", pushed_bytes as i64, "rsp", "r10");
        }
    }

    /// Load `len` (1..=8) bytes at `off` of the object addressed by %rax into
    /// `dest`, guaranteeing no access outside `[off, off + len)`.
    ///
    /// A register-classified aggregate is loaded per eightbyte, and the psABI
    /// says nothing about the bytes of the *last* eightbyte beyond the object's
    /// end -- so an unconditional `movq 8(%rax)` for a 9-byte struct reads 7
    /// bytes the C program does not own.  That is not a cosmetic over-read: an
    /// object can end at a page boundary, and `tests/regression/
    /// large_aggregate_call_copy.c` places its aggregate exactly there, where the
    /// over-read is a SIGSEGV (measured: sizes 1, 7, 9 and 15 faulted with lccc as
    /// the caller, 8 did not -- the exact profile of qword rounding).  The stack
    /// path in `emit_call_stack_args_impl` already refuses to do this, with the
    /// comment that ABI rounding "does not license a read past the C object"; this
    /// is the same rule on the register side.
    ///
    /// The bytes are staged through a 16-byte transient in the outgoing area
    /// (balanced before returning, like the `_Float128` constant path below), so
    /// the sequence needs no register besides %rax -- every other GP is either an
    /// argument register for a call site we cannot see from here or already
    /// staged.  `dest`'s unspecified bytes keep whatever the temp held, which the
    /// psABI allows and `rep movsb`-style copies never promised either.
    fn emit_small_aggregate_eightbyte_load(&mut self, off: usize, len: usize, dest: &str) {
        // MOVQ m64->xmm is SSE2; MOVLPS loads the same eight bytes with SSE1
        // (`-mno-sse2 -msse`; the upper lanes are not part of the eightbyte).
        let load = if !self.isa.simd && dest.starts_with("xmm") {
            "    movlps"
        } else {
            "    movq"
        };
        if len >= 8 {
            // Whole eightbyte: in bounds by construction, and one instruction.
            self.state
                .out
                .emit_instr_mem_reg(load, off as i64, "rax", dest);
            return;
        }
        debug_assert!(
            off + len <= 16,
            "small aggregates span at most two eightbytes"
        );
        // %rax carries the source address through the whole sequence, so it is
        // parked in the transient frame: every ferry load goes through it and
        // destroys the previous value.  Restoring it afterwards keeps whatever the
        // surrounding staging assumed about %rax intact, and the pair is balanced
        // before returning, exactly like the `_Float128` constant staging below.
        self.state.emit("    subq $16, %rsp");
        self.state.emit("    movq %rax, 8(%rsp)");
        debug_assert!(
            len >= 1,
            "a zero-length ferry load copies nothing; the caller must not call that success"
        );
        for NarrowCopy { offset, width } in narrow_copy_plan(len, 4) {
            let mn = narrow_move_mnemonic(width);
            let ferry = eax_ferry(width);
            self.state.emit("    movq 8(%rsp), %rax");
            self.state.emit_fmt(format_args!(
                "    {mn} {}(%rax), {ferry}",
                (off + offset) as i64
            ));
            self.state
                .emit_fmt(format_args!("    {mn} {ferry}, {}(%rsp)", offset as i64));
        }
        self.state.out.emit_instr_mem_reg(load, 0, "rsp", dest);
        self.state.emit("    movq 8(%rsp), %rax");
        // The reload above restores the parked %rax value verbatim, but it is
        // still a %rax write to the epoch analyzer; re-sync the park epoch so
        // the next acc consume does not report the restored value as dead.
        self.state.refresh_acc_park_epoch();
        self.state.emit("    addq $16, %rsp");
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        // Stack args (Phase 2) may have adjusted rsp. Temporarily increase
        // the RSP frame size so rbp-to-rsp offset conversion is correct
        // when loading register arguments from stack slots.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += total_sp_adjust;
        }
        let xmm_regs = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
        ];
        let mut float_count = 0usize;

        // Publish the count of leading SysV GP argument registers this call
        // actually reads — the `# LCCC_CALL_ARGS` marker's data. The liveness
        // oracle's conservative all-six model pins any value homed in an
        // argument register across EVERY call (the hash-chain chase lost its
        // load→compare fold exactly this way when the unroller's profitability
        // gate moved the scratch into %rdx). Computed from the authoritative
        // classification: the highest GP slot index consumed, +1. LP64 only —
        // the i686 regparm/fastcall layouts keep the conservative default.
        if crate::common::types::target_ptr_size() == 8 {
            let mut gp_high = 0usize;
            for cls in arg_classes {
                let span: Option<usize> = match cls {
                    CallArgClass::IntReg { reg_idx } => Some(reg_idx + 1),
                    CallArgClass::I128RegPair { base_reg_idx }
                    | CallArgClass::I64RegPair { base_reg_idx } => Some(base_reg_idx + 2),
                    CallArgClass::StructByValReg { base_reg_idx, size } => {
                        Some(base_reg_idx + size.div_ceil(8))
                    }
                    CallArgClass::StructMixedIntSseReg { int_reg_idx, .. }
                    | CallArgClass::StructMixedSseIntReg { int_reg_idx, .. } => {
                        Some(int_reg_idx + 1)
                    }
                    _ => None,
                };
                if let Some(hi) = span {
                    gp_high = gp_high.max(hi);
                }
            }
            self.state.call_gp_arg_count = gp_high.min(6);
        }

        // ---- Staging-hazard pre-spill (session 25) ----
        // Die-at-birth register coalescing can home a still-needed value in
        // an argument register that an EARLIER argument's staging overwrites:
        // fp_die_at_birth's `cd` accumulator was homed in xmm3 (sharing with
        // the dying `cn`), and staging emitted `movsd %xmm7,%xmm3` (cn, arg4)
        // BEFORE reading cd for arg5 — arg5 then read cn's value ("chain_div
        // returned chain_neg's value").  Detect every argument whose source
        // home is an argument register already written by an earlier-staged
        // argument, and pre-spill those values to a transient stack area
        // BEFORE any staging write.  Symmetric for GPR argument registers.
        //
        // REGISTER-CLASS ARGS ONLY: a Stack-class argument's value was
        // already consumed by `emit_call_stack_args_impl` (the driver runs
        // the stack-argument pushes BEFORE this phase), so no register
        // staging can clobber its read.  Pre-spilling it anyway (the old
        // behavior) bought a dead `movsd %xmmN, K(%rsp)`, the `subq/addq`
        // pair for a hazard area that protected nothing, and — worse — an
        // `addq $K, %rsp` between the register staging and the `call` that
        // the FP liveness oracle's fallback window used to break on (see
        // `call_xmm_reads`): every ≥9-FP-argument call with a stack arg
        // homed in a low xmm bank paid the churn (measured on the PR #584
        // repro: f10 carried a 16-byte hazard area for one stack arg).
        let arg_stages_into_reg = |cls: &CallArgClass| {
            matches!(
                cls,
                CallArgClass::FloatReg { .. }
                    | CallArgClass::IntReg { .. }
                    | CallArgClass::StructSseReg { .. }
                    | CallArgClass::F128SseReg { .. }
                    | CallArgClass::StructXmmReg { .. }
                    | CallArgClass::StructMixedIntSseReg { .. }
                    | CallArgClass::StructMixedSseIntReg { .. }
                    | CallArgClass::I128RegPair { .. }
                    | CallArgClass::StructByValReg { .. }
            )
        };
        let mut written_xmm: [bool; 8] = [false; 8];
        let mut written_gpr: [bool; 6] = [false; 6];
        // (arg index, phys home, is_fp)
        let mut hazards: Vec<(usize, crate::backend::regalloc::PhysReg, bool)> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if let Operand::Value(v) = arg {
                if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    if arg_stages_into_reg(&arg_classes[i]) {
                        if super::emit::is_xmm_reg(phys) {
                            // Allocator XMM homes are PhysReg 20..33 = xmm2..xmm15.
                            let src_idx = phys.0 as i64 - 18;
                            if (0..=7).contains(&src_idx) && written_xmm[src_idx as usize] {
                                hazards.push((i, phys, true));
                            }
                        } else {
                            // Allocatable GPR homes that double as SysV arg regs:
                            // rdi=14→arg0, rsi=15→arg1, rdx=16→arg2, r8=12→arg4,
                            // r9=13→arg5 (rcx/rax are not allocatable homes).
                            let src_idx: Option<usize> = match phys.0 {
                                14 => Some(0),
                                15 => Some(1),
                                16 => Some(2),
                                12 => Some(4),
                                13 => Some(5),
                                _ => None,
                            };
                            if let Some(si) = src_idx {
                                if written_gpr[si] {
                                    hazards.push((i, phys, false));
                                }
                            }
                        }
                    }
                }
            }
            // Mark the targets THIS argument writes (after its own check).
            match &arg_classes[i] {
                CallArgClass::FloatReg { reg_idx } => {
                    if *reg_idx < 8 {
                        written_xmm[*reg_idx] = true;
                    }
                }
                CallArgClass::IntReg { reg_idx } => {
                    if *reg_idx < 6 {
                        written_gpr[*reg_idx] = true;
                    }
                }
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    ..
                } => {
                    if *lo_fp_idx < 8 {
                        written_xmm[*lo_fp_idx] = true;
                    }
                    if let Some(hi) = hi_fp_idx {
                        if *hi < 8 {
                            written_xmm[*hi] = true;
                        }
                    }
                }
                CallArgClass::F128SseReg { reg_idx }
                | CallArgClass::StructXmmReg { reg_idx, .. } => {
                    if *reg_idx < 8 {
                        written_xmm[*reg_idx] = true;
                    }
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                }
                | CallArgClass::StructMixedSseIntReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                } => {
                    if *fp_reg_idx < 8 {
                        written_xmm[*fp_reg_idx] = true;
                    }
                    if *int_reg_idx < 6 {
                        written_gpr[*int_reg_idx] = true;
                    }
                }
                // Register-pair classes: both the base register and its
                // successor are written by the staging (I128RegPair loads
                // both halves; StructByValReg stages an aligned pair).
                // Without this marking the dynamic arg-hazard pre-spill
                // could not see the second register of the pair as written
                // (defense-in-depth: the RA's later_arg_values exclusion is
                // the primary contract).
                CallArgClass::I128RegPair { base_reg_idx }
                | CallArgClass::StructByValReg { base_reg_idx, .. } => {
                    if *base_reg_idx < 6 {
                        written_gpr[*base_reg_idx] = true;
                    }
                    if *base_reg_idx + 1 < 6 {
                        written_gpr[*base_reg_idx + 1] = true;
                    }
                }
                _ => {}
            }
        }
        let mut hazard_slot: crate::common::fx_hash::FxHashMap<usize, i64> =
            crate::common::fx_hash::FxHashMap::default();
        let hazard_area = ((hazards.len() * 8 + 15) / 16) * 16;
        if hazard_area > 0 {
            self.state
                .emit_fmt(format_args!("    subq ${}, %rsp", hazard_area));
            // The pre-spill area moves rsp: every rsp-relative slot read
            // emitted during the staging loop below (emit_fp_operand_to_xmm,
            // operand_to_rax, ... all go through slot_ref) must see the
            // enlarged frame, or they read hazard_area bytes BELOW their
            // slot. Under rsp addressing this returned stale/garbage args:
            // printf("%f %f %f %f", s1, s2, t1, t2) printed t1's value as
            // arg1 and stack garbage for args 2-3. rbp-addressed frames are
            // unaffected (slot_ref ignores rsp there), which is why the
            // session-25 hazard tests missed it.
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += hazard_area as i64;
            }
            for (k, (arg_i, phys, is_fp)) in hazards.iter().enumerate() {
                let off = (k * 8) as i64;
                hazard_slot.insert(*arg_i, off);
                let name = super::emit::phys_reg_name(*phys);
                if *is_fp {
                    // 8-byte FP spill; MOVLPS is the SSE1 form of MOVSD here.
                    let mov = if self.isa.simd { "movsd" } else { "movlps" };
                    self.state
                        .emit_fmt(format_args!("    {mov} %{}, {}(%rsp)", name, off));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, {}(%rsp)", name, off));
                }
            }
        }

        let mut previous_arg_wrote_rcx = false;
        for (i, arg) in args.iter().enumerate() {
            // %rcx is both the secondary value cache and SysV's fourth GP
            // argument. Staging an argument into it invalidates the cached
            // identity, even if that write bypasses operand_to_reg. Defer the
            // invalidation until AFTER the previous argument has consumed its
            // source, and put it before every next argument (including the
            // hazard/global-address paths that continue early below).
            // Otherwise a later aggregate pointer can be "reloaded" from the
            // scalar fourth argument and dereferenced (CC-O0CALL-1).
            if previous_arg_wrote_rcx {
                self.state.reg_cache.invalidate_sec();
            }
            previous_arg_wrote_rcx = match arg_classes[i] {
                CallArgClass::IntReg { reg_idx } => reg_idx == 3,
                CallArgClass::I128RegPair { base_reg_idx }
                | CallArgClass::I64RegPair { base_reg_idx } => {
                    base_reg_idx <= 3 && 3 < base_reg_idx + 2
                }
                CallArgClass::StructByValReg { base_reg_idx, size } => {
                    base_reg_idx <= 3 && 3 < base_reg_idx + size.div_ceil(8)
                }
                CallArgClass::StructMixedIntSseReg { int_reg_idx, .. }
                | CallArgClass::StructMixedSseIntReg { int_reg_idx, .. } => int_reg_idx == 3,
                _ => false,
            };
            // Hazard arguments read their pre-spilled slot instead of the
            // (already-overwritten) argument-register home.
            if let Some(&off) = hazard_slot.get(&i) {
                match &arg_classes[i] {
                    CallArgClass::FloatReg { reg_idx } => {
                        let mov = if self.isa.simd { "movsd" } else { "movlps" };
                        self.state.emit_fmt(format_args!(
                            "    {mov} {}(%rsp), %{}",
                            off, xmm_regs[*reg_idx]
                        ));
                        float_count += 1;
                        continue;
                    }
                    CallArgClass::IntReg { reg_idx } => {
                        self.state.emit_fmt(format_args!(
                            "    movq {}(%rsp), %{}",
                            off, X86_ARG_REGS[*reg_idx]
                        ));
                        continue;
                    }
                    _ => {}
                }
            }
            match arg_classes[i] {
                CallArgClass::I128RegPair { base_reg_idx } => {
                    let lo_reg = X86_ARG_REGS[base_reg_idx];
                    let hi_reg = X86_ARG_REGS[base_reg_idx + 1];
                    // Load 128-bit value directly into the target register pair,
                    // avoiding operand_to_rax_rdx which clobbers rax and rdx
                    // (potentially overwriting previously-assigned arguments).
                    match arg {
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Load both halves directly from the stack slot
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    movq", slot.0, lo_reg);
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    movq", slot.0 + 8, hi_reg);
                            } else {
                                // No slot: zero both halves
                                self.state
                                    .emit_fmt(format_args!("    xorq %{}, %{}", lo_reg, lo_reg));
                                self.state
                                    .emit_fmt(format_args!("    xorq %{}, %{}", hi_reg, hi_reg));
                            }
                        }
                        Operand::Const(c) => {
                            match c {
                                IrConst::I128(v) => {
                                    let low = *v as u64;
                                    let high = (*v >> 64) as u64;
                                    self.state.emit_fmt(format_args!(
                                        "    movabsq ${}, %{}",
                                        low as i64, lo_reg
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    movabsq ${}, %{}",
                                        high as i64, hi_reg
                                    ));
                                }
                                IrConst::Zero => {
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        lo_reg, lo_reg
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        hi_reg, hi_reg
                                    ));
                                }
                                _ => {
                                    // Smaller constant: load into lo_reg via rax, zero hi_reg
                                    self.operand_to_rax(arg);
                                    self.state.out.emit_instr_reg_reg("    movq", "rax", lo_reg);
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        hi_reg, hi_reg
                                    ));
                                }
                            }
                        }
                    }
                }
                CallArgClass::StructByValReg { base_reg_idx, size } => {
                    self.operand_to_rax(arg);
                    let lo_reg = X86_ARG_REGS[base_reg_idx];
                    // Low eightbyte first: for sizes below 8 it clobbers %rax,
                    // which is only read again when there is no high eightbyte.
                    self.emit_struct_eightbyte_exact(size, 0, lo_reg);
                    if size > 8 {
                        let hi_reg = X86_ARG_REGS[base_reg_idx + 1];
                        self.emit_struct_eightbyte_exact(size, 8, hi_reg);
                    }
                }
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    size,
                } => {
                    self.operand_to_rax(arg);
                    self.emit_small_aggregate_eightbyte_load(0, size.min(8), xmm_regs[lo_fp_idx]);
                    float_count += 1;
                    if let Some(hi) = hi_fp_idx {
                        self.emit_small_aggregate_eightbyte_load(
                            8,
                            size.saturating_sub(8).min(8),
                            xmm_regs[hi],
                        );
                        float_count += 1;
                    }
                }
                CallArgClass::StructXmmReg { reg_idx, size } => {
                    // A `[SSE, SSEUP...]` aggregate is exactly one vector
                    // register wide (16 / 32 / 64 bytes): load it through
                    // its address into that register.
                    self.operand_to_rax(arg);
                    let (mov, reg) = vector_reg_move(size, reg_idx);
                    self.state
                        .emit_fmt(format_args!("    {mov} (%rax), %{reg}"));
                    float_count += 1;
                }
                CallArgClass::F128SseReg { reg_idx } => {
                    // _Float128 / _Decimal128: the 16 bytes go in ONE XMM register.
                    self.emit_f128_sse_operand_to_xmm(arg, xmm_regs[reg_idx]);
                    float_count += 1;
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    size,
                } => {
                    self.operand_to_rax(arg);
                    self.emit_small_aggregate_eightbyte_load(
                        8,
                        size.saturating_sub(8).min(8),
                        xmm_regs[fp_reg_idx],
                    );
                    float_count += 1;
                    self.emit_small_aggregate_eightbyte_load(
                        0,
                        size.min(8),
                        X86_ARG_REGS[int_reg_idx],
                    );
                }
                CallArgClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    size,
                } => {
                    self.operand_to_rax(arg);
                    self.emit_small_aggregate_eightbyte_load(
                        8,
                        size.saturating_sub(8).min(8),
                        X86_ARG_REGS[int_reg_idx],
                    );
                    self.emit_small_aggregate_eightbyte_load(0, size.min(8), xmm_regs[fp_reg_idx]);
                    float_count += 1;
                }
                CallArgClass::FloatReg { reg_idx } => {
                    match arg {
                        Operand::Const(IrConst::F64(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::F64, xmm_regs[reg_idx]);
                        }
                        Operand::Const(IrConst::F32(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::F32, xmm_regs[reg_idx]);
                        }
                        // Decimal carriers: bit-exact SSE moves.
                        Operand::Const(IrConst::D64(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::D64, xmm_regs[reg_idx]);
                        }
                        Operand::Const(IrConst::D32(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::D32, xmm_regs[reg_idx]);
                        }
                        _ => {
                            // Width-exact staging. The value's FRESH home
                            // decides the cheapest correct move:
                            //   * XMM home → one direct scalar move (the VEX
                            //     3-operand form reads only the source, same
                            //     law as emit_fp_operand_to_xmm's direct
                            //     path) — the old unconditional
                            //     `operand_to_rax` + `movq %rax, %xmmN`
                            //     shuttle paid TWO instructions and a GPR
                            //     bit-cast round trip for a value that was
                            //     already vector-resident (the printf
                            //     staging relay the copy-budget relay
                            //     counter caught);
                            //   * GPR home (a bit-cast carrier) → one
                            //     movd/movq straight from that GPR, not
                            //     through %rax;
                            //   * otherwise the accumulator shuttle: when the
                            //     value is acc-resident, `operand_to_rax`
                            //     emits nothing and the pair is one move —
                            //     the historical single-instruction case,
                            //     preserved.
                            let ty = arg_types.get(i).copied().unwrap_or(IrType::F64);
                            let mut staged = false;
                            if let Operand::Value(v) = arg {
                                if let Some(home) = self.fresh_home_of(v.0) {
                                    if super::emit::is_xmm_reg(home) {
                                        let name = super::emit::phys_reg_name(home);
                                        if name != xmm_regs[reg_idx] {
                                            // Full-register packed copy:
                                            // the same low-128 bits as the
                                            // 3-operand `vmovsd %s,%s,%d`,
                                            // but eliminated at rename
                                            // instead of a port-5 shuffle.
                                            // D32 is a 4-byte datum like F32.
                                            // MOVAPD is SSE2: without it
                                            // (`-mno-sse2 -msse`, an 8-byte
                                            // vector argument) MOVAPS copies
                                            // the same 128 bits.
                                            let f32ish = matches!(ty, IrType::F32 | IrType::D32)
                                                || !self.isa.simd;
                                            let mv = match (f32ish, self.isa.avx) {
                                                (true, true) => "vmovaps",
                                                (true, false) => "movaps",
                                                (false, true) => "vmovapd",
                                                (false, false) => "movapd",
                                            };
                                            self.state.emit_fmt(format_args!(
                                                "    {} %{}, %{}",
                                                mv, name, xmm_regs[reg_idx]
                                            ));
                                        }
                                        staged = true;
                                    } else {
                                        let gpr = super::emit::phys_reg_name(home);
                                        let gpr32 = super::emit::phys_reg_name_32(home);
                                        if ty == IrType::U16 {
                                            self.state.emit_fmt(format_args!(
                                                "    movl %{}, %eax",
                                                gpr32
                                            ));
                                            self.state.emit("    movzwl %ax, %eax");
                                            self.emit_scalar_gpr_to_xmm(
                                                "rax",
                                                "eax",
                                                xmm_regs[reg_idx],
                                                true,
                                            );
                                        } else {
                                            self.emit_scalar_gpr_to_xmm(
                                                gpr,
                                                gpr32,
                                                xmm_regs[reg_idx],
                                                matches!(ty, IrType::F32 | IrType::D32),
                                            );
                                        }
                                        staged = true;
                                    }
                                }
                            }
                            if !staged {
                                self.operand_to_rax(arg);
                                if ty == IrType::U16 {
                                    self.state.emit("    movzwl %ax, %eax");
                                }
                                self.emit_scalar_gpr_to_xmm(
                                    "rax",
                                    "eax",
                                    xmm_regs[reg_idx],
                                    matches!(ty, IrType::F32 | IrType::D32 | IrType::U16),
                                );
                            }
                        }
                    }
                    float_count += 1;
                }
                CallArgClass::IntReg { reg_idx } => {
                    let target_reg = X86_ARG_REGS[reg_idx];
                    // Rematerializable call argument: a GlobalAddr root with
                    // no home (neither a register assignment nor a stack
                    // slot) rebuilds by LEA. Stage it DIRECTLY into the
                    // argument register — routing through operand_to_rax
                    // would emit a LEA-into-%rax plus mov relay that the
                    // peephole cannot fold through the call barrier. Same
                    // def lookup as value_to_reg_inner's rebuild (the name is
                    // cloned to end the immutable borrow before emitting);
                    // TLS/GOT/absolute forms are handled by the shared
                    // helper, which also owns cache hygiene.
                    if let Operand::Value(v) = arg {
                        if !self.reg_assignments.contains_key(&v.0)
                            && self.state.get_slot(v.0).is_none()
                        {
                            let gaddr_name = self.get_defining_instruction(v.0).and_then(|inst| {
                                if let Instruction::GlobalAddr { name, .. } = inst {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            });
                            if let Some(name) = gaddr_name {
                                self.emit_global_addr_into_reg(&name, target_reg);
                                continue;
                            }
                        }
                    }
                    // Register-direct: callee-saved regs and constants bypass rax.
                    // Skip when it would create a round-trip (e.g., rdi→rbx→rdi)
                    // that the peephole would eliminate along with the param store.
                    let did_direct = if let Operand::Value(v) = arg {
                        if let Some(&phys) = self.reg_assignments.get(&v.0) {
                            if phys.0 >= 1 && phys.0 <= 6 {
                                // Check for round-trip: if this callee-saved reg was
                                // loaded from the SAME arg reg we're targeting, skip
                                let is_round_trip = self
                                    .param_source_regs
                                    .get(&phys.0)
                                    .is_some_and(|&src| src == target_reg);
                                if !is_round_trip {
                                    let src = super::emit::phys_reg_name(phys);
                                    self.state
                                        .out
                                        .emit_instr_reg_reg("    movq", src, target_reg);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else if let Operand::Const(c) = arg {
                        if let Some(imm) = c.to_i64() {
                            if imm == 0 {
                                let target_32 = match target_reg {
                                    "rdi" => "edi",
                                    "rsi" => "esi",
                                    "rdx" => "edx",
                                    "rcx" => "ecx",
                                    "r8" => "r8d",
                                    "r9" => "r9d",
                                    _ => target_reg,
                                };
                                self.state.emit_fmt(format_args!(
                                    "    xorl %{}, %{}",
                                    target_32, target_32
                                ));
                            } else {
                                self.state
                                    .out
                                    .emit_instr_imm_reg("    movq", imm, target_reg);
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !did_direct {
                        self.operand_to_rax(arg);
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", "rax", target_reg);
                    }
                }
                _ => {}
            }
        }
        if float_count > 0 && self.state.pending_sibcall.is_none() {
            // The census is ABI payload only for VARIADIC callees, and the
            // GP dead-write pass deletes it for non-variadic ones — except
            // at a sibling call, where the late passes have no call text to
            // model the site from and keep it. A sibcall is non-variadic by
            // construction (the arm refuses variadic callees), so skip the
            // write outright there: the `# LCCC_CALL_FP` marker below still
            // publishes the count for the FP liveness oracle.
            self.state
                .out
                .emit_instr_imm_reg("    movb", float_count as i64, "al");
        } else if self.state.call_is_variadic && !(self.skip_rax_setup && self.no_sse) {
            // %al reports the number of live SSE argument registers to a
            // VARIADIC callee (SysV AMD64 3.5.7). It is meaningless for a
            // prototyped non-variadic callee, and GCC emits nothing there --
            // lccc used to zero %eax before every indirect call, costing an
            // instruction and a false dependency on %rax at each site.
            //
            // -mskip-rax-setup additionally drops it for variadic callees,
            // sound only when no SSE argument register can be live, which
            // -mno-sse guarantees. A real float argument always wins (the
            // `float_count > 0` arm above).
            self.state.emit("    xorl %eax, %eax");
        }
        // Arm the FP-count authority marker: the exact number of leading
        // SSE argument registers this call reads (every FloatReg/
        // StructSseReg/F128/mixed-fp staging incremented `float_count`, the
        // hazard-restore path included). The `movb $N, %al` census above
        // carries the same number, but the GP dead-write pass legitimately
        // deletes it for non-variadic callees (%al is dead there) — after
        // which the FP liveness oracle's window heuristic is the only
        // source left, and it under-collects across hazard-area releases
        // and deep relay chains (PR #584: a 10-double call lost 7 of its 8
        // register arguments to `eliminate_dead_vector_copies`). The
        // marker is a comment: free to keep, exact, and immune to every
        // value-shape pass. Emitted right after the call text by
        // `emit_call_instruction_impl` (same discharge contract as
        // `call_gp_arg_count`).
        // Microsoft FP register numbers follow argument positions, not a
        // compact FP census. Preserve the whole live prefix through the
        // highest occupied position in the count-based peephole contract.
        if self.state.call_calling_convention
            == crate::common::types::CallingConvention::MicrosoftX64
        {
            float_count = arg_classes
                .iter()
                .filter_map(|c| {
                    if let CallArgClass::FloatReg { reg_idx } = c {
                        Some(reg_idx + 1)
                    } else {
                        None
                    }
                })
                .max()
                .unwrap_or(0);
        }
        self.state.call_fp_arg_count = Some(float_count.min(8));
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        // Release the staging-hazard pre-spill area (all hazard args have
        // been staged from it; nothing beyond this point reads it).
        if hazard_area > 0 {
            self.state
                .emit_fmt(format_args!("    addq ${}, %rsp", hazard_area));
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= hazard_area as i64;
            }
        }
        if self.state.call_calling_convention
            == crate::common::types::CallingConvention::MicrosoftX64
            && self.state.call_is_variadic
        {
            for class in arg_classes {
                if let CallArgClass::FloatReg { reg_idx } = *class {
                    const GP: [&str; 4] = ["rcx", "rdx", "r8", "r9"];
                    self.state
                        .emit_fmt(format_args!("    movq %xmm{}, %{}", reg_idx, GP[reg_idx]));
                }
            }
            self.state.call_gp_arg_count = 6;
        }
        // Restore the original RSP frame size.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= total_sp_adjust;
        }
    }

    /// Clean dirty upper YMM state before an ordinary call, as GCC 16.2 and
    /// Clang 23.1.4 do: a legacy-SSE callee otherwise runs every XMM write
    /// with a merge and a false dependency on the upper half (Skylake and
    /// later, including the Raptor Lake target) or pays a ~70-cycle state
    /// transition (Haswell/Broadwell). Sound because SysV keeps no vector
    /// value in a register across a call: every XMM/YMM register is
    /// caller-saved and the allocator refuses call-spanning vector homes
    /// (`spans_any_call`). A YMM/ZMM argument is the one live upper half and
    /// suppresses it (`call_passes_wide_vector`).
    ///
    /// `dirty_upper_ymm` stays set after the call. It is tracked in emission
    /// order, not per path, so clearing it here would let a path that
    /// bypasses this call return dirty; the epilogue `vzeroupper` that may
    /// follow is cheap and never wrong.
    fn emit_vzeroupper_before_call(&mut self) {
        if self.state.dirty_upper_ymm && !self.state.call_passes_wide_vector {
            self.state.emit("    vzeroupper");
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        _indirect: bool,
        stack_arg_space: usize,
    ) {
        if self.state.call_calling_convention
            == crate::common::types::CallingConvention::MicrosoftX64
        {
            self.reload_spilled_indirect_fptr_to_r10(stack_arg_space);
            self.state.emit("    subq $32, %rsp");
            self.state.call_shadow_active = true;
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 32;
            }
        }
        // Lazy flush: the call clobbers all XMM registers — a pending
        // deferred vector-result store must hit its slot first.
        self.flush_pending_vec_store_impl();
        // Sibling call (TODO-3 / §5b): argument setup ran unchanged; the
        // `call` is replaced by the block's `Return` emitting
        // `<epilogue>; jmp target`. Resolve the target exactly as the
        // non-sibcall path below would have spelled it (PLT policy,
        // paravirt memory-slot folding, `%r10` for value-based indirects)
        // so the emitted `jmp` keeps byte-for-byte the binding semantics
        // of the `call` it replaces. The call TEXT is suppressed but the
        // liveness-marker contract below still discharges — the late text
        // passes model this site from the markers, and without them the
        // dead-write eliminations around it (the `movb $N, %al` census,
        // param-homing moves) silently stop firing.
        let preserve_upper_vector_lanes = match &self.state.pending_sibcall {
            Some(SibcallPending::Armed {
                preserve_upper_vector_lanes,
            }) => Some(*preserve_upper_vector_lanes),
            _ => None,
        };
        // A sibcall cleans at its `jmp` (returns.rs), not here.
        if preserve_upper_vector_lanes.is_none() {
            // The frame planner dropped the call-alignment pad only after
            // predicting that every call of this function converts; a real
            // `call` here would run with %rsp misaligned. Fail closed.
            assert!(
                self.func_has_non_tail_calls || !self.func_has_calls,
                "x86-64 frame planned without a call-alignment pad, but a call site of `{}` \
                 did not become a sibling call",
                self.state.current_func_name
            );
            self.emit_vzeroupper_before_call();
        }
        if let Some(preserve_upper_vector_lanes) = preserve_upper_vector_lanes {
            let target = if let Some(name) = direct_name {
                if self.state.needs_got_call(name) {
                    let n = name.split('@').next().unwrap_or(name);
                    SibcallTarget::RipSlot(format!("{}@GOTPCREL", n))
                } else if self.state.needs_plt(name) {
                    let n = name.split('@').next().unwrap_or(name);
                    SibcallTarget::Direct(format!("{}@PLT", n))
                } else {
                    SibcallTarget::Direct(name.to_string())
                }
            } else if let Some(fp) = func_ptr {
                match self.try_resolve_indirect_call_global(fp) {
                    Some(ResolvedIndirectCall::MemorySlot(sym, off)) => {
                        let sym_str = if off == 0 {
                            sym.clone()
                        } else if off > 0 {
                            format!("{}+{}", sym, off)
                        } else {
                            format!("{}{}", sym, off)
                        };
                        SibcallTarget::RipSlot(sym_str)
                    }
                    Some(ResolvedIndirectCall::Direct(sym, off)) => {
                        if off == 0 && self.state.needs_got_call(&sym) {
                            SibcallTarget::RipSlot(format!("{}@GOTPCREL", sym))
                        } else if off == 0 && self.state.needs_plt(&sym) {
                            SibcallTarget::Direct(format!("{}@PLT", sym))
                        } else if off == 0 {
                            SibcallTarget::Direct(sym.clone())
                        } else if off > 0 {
                            SibcallTarget::Direct(format!("{}+{}", sym, off))
                        } else {
                            SibcallTarget::Direct(format!("{}{}", sym, off))
                        }
                    }
                    // Value-based indirect: Phase 0 already loaded the
                    // target into %r10 (emit_call_spill_fptr_impl), or parked
                    // it on the stack when %r10 held a call argument; the
                    // epilogue does not touch caller-saved registers.
                    None => {
                        // The sibcall arming guards (arm_sibcall_candidate_impl)
                        // refuse every stack-class argument and every variadic
                        // /sret/regparm case, and dynamic realignment exists
                        // solely for >16-aligned STACK arguments, so both are
                        // unreachable here. The parked-fptr reload therefore
                        // always takes its zero-arg self-reclaiming branch:
                        // assert the full reclaim so a future relaxation of an
                        // arming guard fails loudly instead of tail-jumping
                        // with the transient 16-byte slot still on %rsp.
                        debug_assert!(!self.dyn_align_cleanup);
                        debug_assert_eq!(stack_arg_space, 0);
                        self.reload_spilled_indirect_fptr_to_r10(stack_arg_space);
                        debug_assert!(
                            !self.indirect_fptr_on_stack,
                            "sibling call must reclaim the parked fptr slot before jmp *r10"
                        );
                        SibcallTarget::Reg("r10")
                    }
                }
            } else {
                // Unreachable: the driver only arms sites whose emission
                // reaches the generic `emit_call` with a direct name or a
                // function pointer. Fail closed rather than emit nothing.
                self.state.pending_sibcall = None;
                return;
            };
            self.state.pending_sibcall = Some(SibcallPending::Ready {
                target,
                preserve_upper_vector_lanes,
            });
        } else if let Some(name) = direct_name {
            if self.state.needs_got_call(name) {
                let n = name.split('@').next().unwrap_or(name);
                self.state
                    .emit_fmt(format_args!("    call *{}@GOTPCREL(%rip)", n));
            } else if self.state.needs_plt(name) {
                // Versioned symbols (`printf@GLIBC_2.2.5`, bound by .symver)
                // must use the base name in @PLT references — GAS 2.47 rejects
                // `sym@ver@PLT` (GAS-oracle). The linker resolves the version.
                let n = name.split('@').next().unwrap_or(name);
                self.state.emit_fmt(format_args!("    call {}@PLT", n));
            } else {
                self.state.out.emit_call(name);
            }
        } else if let Some(fp) = func_ptr {
            // Paravirt/global fast-path: if func_ptr resolves to a global+offset
            // (e.g. pv_ops+72), emit `call *sym+off(%rip)` directly. This produces
            // the `ff 15 disp32` encoding with R_X86_64_PC32 reloc that
            // arch/x86/kernel/paravirt.c:apply_alternatives expects to patch.
            // Without this, we would emit `mov $sym, %r15; call *off(%r15)`
            // (41 ff 57) which is not patchable and breaks boot.
            if let Some(target) = self.try_resolve_indirect_call_global(fp) {
                // Construct `sym+off` string for GAS. Use `sym` alone when off==0.
                let (sym, off) = match &target {
                    ResolvedIndirectCall::MemorySlot(sym, off)
                    | ResolvedIndirectCall::Direct(sym, off) => (sym.clone(), *off),
                };
                let sym_str = if off == 0 {
                    sym.clone()
                } else if off > 0 {
                    format!("{}+{}", sym, off)
                } else {
                    format!("{}{}", sym, off) // off negative includes '-'
                };
                match target {
                    // Memory-indirect through the slot: GAS produces ff 15 with a
                    // PC32 reloc (the paravirt patchable form). Raw emit avoids
                    // any PLT/GOT logic.
                    ResolvedIndirectCall::MemorySlot(..) => {
                        self.state
                            .emit_fmt(format_args!("    call *{}({})", sym_str, "%rip"));
                    }
                    // The resolved value IS the target: direct call, with the
                    // same PLT handling as a syntactically direct call. An
                    // offset into the callee (`&func + off`, computed-goto
                    // style block addresses) is never interposed — the PLT
                    // entry is only defined for offset 0 — so the offset is
                    // preserved and @PLT is used only for it. (Versioned
                    // symbols never reach this arm: they are rejected in
                    // try_resolve_indirect_call_global and fall back to the
                    // value-based call, which preserves the .symver binding.)
                    ResolvedIndirectCall::Direct(..) => {
                        if off == 0 && self.state.needs_got_call(&sym) {
                            self.state
                                .emit_fmt(format_args!("    call *{}@GOTPCREL(%rip)", sym));
                        } else if off == 0 && self.state.needs_plt(&sym) {
                            self.state.emit_fmt(format_args!("    call {}@PLT", sym));
                        } else {
                            self.state.out.emit_call(&sym_str);
                        }
                    }
                }
            } else {
                // Indirect function pointers are preloaded into r10 by
                // emit_call_spill_fptr_impl before stack arguments are pushed
                // (or parked on the stack when %r10 held a call argument).
                self.reload_spilled_indirect_fptr_to_r10(stack_arg_space);
                self.emit_retpoline_call("r10");
            }
        }
        // Variadic-call marker (same authority contract as the
        // `# LCCC_RET_*` prologue markers): a variadic callee reads %rax
        // (the live SSE register count in %al — SysV AMD64 3.5.7); a
        // prototyped non-variadic callee reads NOTHING from the
        // accumulator. The late text peephole's GP liveness oracle cannot
        // see the callee's signature, so it conservatively read %rax at
        // EVERY call — pinning values in %rax across whole loops and
        // blocking the copy/extension folds (the gzip_crc32 harness kept
        // `movzbl+movq+movb` chains live solely on this false read). The
        // marker lands immediately after the call text (before any
        // subsequent emission), and absent marker ⇒ non-variadic ⇒ %rax
        // unread.
        if self.state.call_is_variadic
            && self.state.call_calling_convention
                == crate::common::types::CallingConvention::Default
        {
            self.emit_call_marker("    # LCCC_VA_CALL".to_string());
        }
        // Call-argument count marker (the same authority contract as the
        // VA marker above): the GP liveness oracle reads this to model
        // exactly the LEADING SysV GP argument registers the callee
        // consumes, instead of conservatively pinning all six across every
        // call. 6 (or an absent marker — hand-written fragments, raw `call`
        // emissions like the i128 helpers) keeps the old conservative read
        // set: fail-closed.
        self.emit_call_marker(format!(
            "    # LCCC_CALL_ARGS {}",
            self.state.call_gp_arg_count.min(6)
        ));
        // Static-chain call marker (same authority contract): a chain call
        // reads %r10 (the nested-callee's static chain, staged by the
        // immediately preceding SetStaticChain emission). Absent marker ⇒
        // the callee cannot read a chain this caller staged ⇒ %r10 scratch
        // values may be proven dead across the call. This is what un-pins
        // %r10 (the RA's most common scratch after rax/rcx/rdx) from every
        // call in the text liveness oracles.
        //
        // ORDERING CONTRACT: the GP liveness oracle scans a TIGHT window
        // (the two lines after the call text) for this marker, so it must
        // stay in the same position the pre-FP-marker emission gave it —
        // [call][VA?][CALL_ARGS][CHAIN?] with CHAIN at n+1/n+2. The
        // LCCC_CALL_FP marker below is therefore emitted LAST.
        if self.state.chain_call {
            self.emit_call_marker("    # LCCC_CHAIN_CALL".to_string());
            self.state.chain_call = false;
        }
        // FP-argument count marker (the same authority contract as
        // LCCC_CALL_ARGS above): the exact number of leading SSE argument
        // registers this call reads, armed by `emit_call_reg_args_impl`
        // from the authoritative classification. Consumed by the FP
        // liveness oracle's `call_xmm_reads` BEFORE its census/window
        // heuristics, so the argument staging can never be modeled dead
        // once the `movb $N, %al` census is dead-eliminated. Absent marker
        // (hand-written fragments, raw libcall emissions) keeps the
        // heuristic path: fail-closed to the census first. Emitted LAST:
        // every earlier marker's consumer scans a tight window measured
        // from the call text, and this one's parser skips sibling markers.
        if let Some(fp) = self.state.call_fp_arg_count {
            self.emit_call_marker(format!("    # LCCC_CALL_FP {}", fp.min(8)));
        }
        // IPA-RA keep set (`ipa_ra`): the kept registers holding a value live
        // across THIS call (nonzero only when every call point of the
        // function is a qualifying direct call to a summarized callee). After
        // CALL_FP: the earlier markers' windows are unchanged and
        // `call_markers` reads the whole contiguous block (at most five
        // lines, inside its window).
        let keep = self.ipa_keep_at(self.state.current_program_point);
        if keep != 0 {
            self.emit_call_marker(format!("    # LCCC_IPA_KEEP {:#06x}", keep));
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// Publish one call-site marker line. At a sibling call (the call text
    /// suppressed, `pending_sibcall` `Ready`) the line is also kept for
    /// `emit_return_impl`, which repeats the marker block after the tail
    /// `jmp` (see `BackendState::sibcall_marker_lines`).
    fn emit_call_marker(&mut self, line: String) {
        if matches!(
            self.state.pending_sibcall,
            Some(SibcallPending::Ready { .. })
        ) {
            self.state.sibcall_marker_lines.push(line.clone());
        }
        self.state.emit(&line);
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        _indirect: bool,
    ) {
        if self.state.call_shadow_active {
            self.state.emit("    addq $32, %rsp");
            self.state.call_shadow_active = false;
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= 32;
            }
        }
        // Dynamically realigned outgoing area: after the call %rsp points at
        // the first stack argument (the anchor). Skip the statically-pushed
        // argument area (16-rounded) to the save slot written by
        // emit_call_stack_args_impl, then restore %rsp EXACTLY to its
        // pre-realignment value — correct even under DynAlloca. %r11 is the
        // scratch — %rax may still carry the callee's return value for Phase
        // 6's result store, which runs after this cleanup.
        if self.dyn_align_cleanup {
            self.dyn_align_cleanup = false;
            let area = (stack_arg_space + 15) & !15;
            if area > 0 {
                self.state
                    .out
                    .emit_instr_imm_reg("    addq", area as i64, "rsp");
            }
            self.state.emit("    movq (%rsp), %r11");
            self.state.emit("    movq %r11, %rsp");
            if self.indirect_fptr_on_stack {
                self.indirect_fptr_on_stack = false;
                self.state.emit("    addq $16, %rsp");
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size -= 16;
                }
            }
            return;
        }
        let fptr_extra = if self.indirect_fptr_on_stack {
            self.indirect_fptr_on_stack = false;
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= 16;
            }
            16
        } else {
            0
        };
        let need_align_pad = stack_arg_space % 16 != 0;
        let total_cleanup = stack_arg_space + if need_align_pad { 8 } else { 0 } + fptr_extra;
        if total_cleanup > 0 {
            self.state
                .out
                .emit_instr_imm_reg("    addq", total_cleanup as i64, "rsp");
        }
    }

    /// Write a scalar 16-byte SSE-carrier argument into its ALREADY RESERVED
    /// outgoing stack slot. The slot reservation and RSP-frame accounting are
    /// owned by the stack-argument walk so value slots remain addressable.
    fn emit_f128_sse_stack_arg(&mut self, op: &Operand) {
        // A deferred vector result may still live in xmm0 (or need %rax for
        // its destination). Materialize that store before either scratch is
        // reused for this argument. RSP/frame accounting already includes the
        // reserved outgoing slot, so a slot-relative flush remains correct.
        self.flush_pending_vec_store_impl();
        match op {
            Operand::Value(_) => {
                self.emit_f128_sse_operand_to_xmm(op, "xmm0");
                self.state
                    .emit_fmt(format_args!("    {} %xmm0, (%rsp)", self.xmm_move(false)));
            }
            Operand::Const(c) => {
                let bits = crate::backend::call_abi::f128_sse_const_image(*c);
                for (offset, half) in [(0usize, bits as u64), (8, (bits >> 64) as u64)] {
                    if half == 0 {
                        self.state
                            .emit_fmt(format_args!("    movq $0, {offset}(%rsp)"));
                    } else {
                        self.state
                            .out
                            .emit_instr_imm_reg("    movabsq", half as i64, "rax");
                        self.state
                            .emit_fmt(format_args!("    movq %rax, {offset}(%rsp)"));
                    }
                }
                // A nonzero immediate used %rax; invalidating unconditionally
                // is conservative for the all-zero form and keeps the cache
                // contract local to this emission helper.
                self.state.reg_cache.invalidate_acc();
            }
        }
        self.state.invalidate_vec_peephole();
    }

    /// Load a 16-byte SSE + SSEUP value (it travels in ONE XMM register) into
    /// `xmm`: a scalar `_Float128` / `_Decimal128` argument or return, or the
    /// returned bytes of a one-XMM aggregate. Such a value is always homed in
    /// a 16-byte slot; a constant is materialized bit-exact through a stack
    /// scratch, so no other XMM or argument register is disturbed. Aggregate
    /// ARGUMENTS use `StructXmmReg` (operand = address) instead, and
    /// `long double` (X87 class) never travels in XMM.
    pub(super) fn emit_f128_sse_operand_to_xmm(&mut self, op: &Operand, xmm: &str) {
        let bits: u128 = match op {
            Operand::Value(v) => {
                let slot = crate::backend::call_abi::require_f128_sse_value_slot(
                    self.state.get_slot(v.0),
                    v.0,
                );
                self.state.out.emit_instr_rbp_reg(
                    &format!("    {}", self.xmm_move(false)),
                    slot.0,
                    xmm,
                );
                return;
            }
            Operand::Const(c) => {
                let bits = crate::backend::call_abi::f128_sse_const_image(*c);
                if bits == 0 {
                    // Do not use IrConst::is_zero here: -0.0 must retain its
                    // sign bit. Only the all-zero 16-byte image takes this arm.
                    let zero = if self.isa.simd { "pxor" } else { "xorps" };
                    self.state
                        .emit_fmt(format_args!("    {zero} %{xmm}, %{xmm}"));
                    return;
                }
                bits
            }
        };
        let lo = bits as u64;
        let hi = (bits >> 64) as u64;
        self.state.emit("    subq $16, %rsp");
        self.state
            .out
            .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
        self.state.emit("    movq %rax, (%rsp)");
        self.state
            .out
            .emit_instr_imm_reg("    movabsq", hi as i64, "rax");
        self.state.emit("    movq %rax, 8(%rsp)");
        self.state
            .emit_fmt(format_args!("    {} (%rsp), %{xmm}", self.xmm_move(false)));
        // The two movabsq materializations destroyed %rax; the cache must
        // not claim the old value (debug-%rax epoch discipline).
        self.state.reg_cache.invalidate_acc();
        self.state.emit("    addq $16, %rsp");
    }

    pub(super) fn set_call_ret_eightbyte_classes_impl(
        &mut self,
        classes: &[crate::common::types::EightbyteClass],
    ) {
        self.call_ret_classes = classes.to_vec();
    }

    pub(super) fn set_call_ret_is_f128_sse_impl(&mut self, is_f128: bool) {
        self.call_ret_is_f128_sse = is_f128;
    }

    /// An aggregate that came back in ONE `%ymm0` / `%zmm0` (the x86-64 AVX
    /// ABI level for `__m256` / `__m512`): store the register through the
    /// caller's result buffer -- the sret-shaped call's hidden argument 0 --
    /// and hand `dest` that address, which is what the sret shape's pointer
    /// result means to the rest of the IR. `%rax` is scratch here: the
    /// callee returned nothing in it.
    pub(super) fn emit_call_store_vector_return_impl(
        &mut self,
        buffer: &Operand,
        bytes: usize,
        dest: Option<&Value>,
    ) {
        self.state.reg_cache.invalidate_all();
        // The buffer operand may live in a caller-saved home that the call
        // clobbered and the Phase 2b restore would only reload afterwards:
        // restore first (idempotent; the later generic restore is a no-op).
        self.emit_post_call_restore_caller_regs();
        self.operand_to_rax(buffer);
        let (mov, reg) = vector_reg_move(bytes, 0);
        self.state
            .emit_fmt(format_args!("    {mov} %{reg}, (%rax)"));
        // The whole SSE file is caller-saved; nothing cached survives a
        // call, and the pending-vector-store peephole must not see a stale
        // image of the buffer.
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if let Some(dest) = dest {
            self.store_rax_to(dest);
        }
    }

    pub(super) fn arm_sibcall_candidate_impl(&mut self, info: &CallInfo) {
        // One conversion in flight at a time; the driver arms at most one
        // site per block and `emit_return_impl` consumes it before the
        // next block begins. A stale `Armed` here means an aborted emission
        // path — refuse to stack a second one on top of it.
        if self.state.pending_sibcall.is_some() {
            return;
        }
        if let Some(preserve_upper_vector_lanes) = self.sibcall_abi_admits(info) {
            self.state.pending_sibcall = Some(SibcallPending::Armed {
                preserve_upper_vector_lanes,
            });
        }
    }

    /// The sibling-call ABI refusals of a call site, as a pure predicate:
    /// `Some(preserve_upper_vector_lanes)` when the site may become
    /// `<epilogue>; jmp target`. The arming above and the frame planner's
    /// prediction (`calculate_stack_space_impl`, which drops the 8-byte
    /// call-alignment pad of a function whose every call converts) share it,
    /// so the two can never disagree.
    pub(super) fn sibcall_abi_admits(&self, info: &CallInfo) -> Option<bool> {
        if info.calling_convention != crate::common::types::CallingConvention::Default
            || self.current_func.is_some_and(|f| unsafe {
                (*f).calling_convention != crate::common::types::CallingConvention::Default
            })
        {
            return None;
        }

        // i686: cdecl stages every argument through the outgoing stack area
        // (see the `%esp`-relative staging in the probe for PR #255), so the
        // zero-stack-argument premise of §5b never holds there. Refused
        // wholesale until an i686-specific proof exists; the guard below
        // (which classifies through the x86-64 SysV config) must not be
        // allowed to green-light a 32-bit target.
        if crate::common::types::target_is_32bit() {
            return None;
        }
        // The remaining ABI refusals from the §5b design: variadic callees
        // read `%al`, sret/fastcall take unaudited argument paths, and the
        // kernel return-thunk build replaces `ret` with a thunk the
        // conversion would bypass.
        if info.is_variadic || info.is_sret || info.is_fastcall || info.regparm.is_some() {
            return None;
        }
        if self.state.function_return_thunk {
            return None;
        }
        // Zero-stack-argument guard: classify exactly as the emission will
        // (same `CallAbiConfig`, same arrays) and refuse unless every
        // argument lands in a register. This is the soundness core: with
        // no stack-passed arguments, the callee reads nothing below `%rsp`,
        // so the 16-byte window shift between `call` and `jmp` cannot
        // change any byte the callee observes (pr23324 stays impossible).
        let config = self.call_abi_config_impl();
        let classes = crate::backend::call_abi::classify_call_args(
            &info.args,
            &info.arg_types,
            &info.struct_arg_sizes,
            &info.struct_arg_aligns,
            &info.struct_arg_classes,
            &info.struct_arg_riscv_float_classes,
            &info.struct_arg_is_f128_sse,
            info.is_variadic,
            &config,
        );
        if classes
            .iter()
            .any(crate::backend::call_abi::CallArgClass::is_stack)
        {
            return None;
        }
        Some(classes.iter().any(|class| {
            matches!(
                class,
                crate::backend::call_abi::CallArgClass::StructXmmReg { size, .. }
                    if *size > 16
            )
        }))
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        // Sibling call (TODO-3): the result is never materialised here —
        // the tail callee's exit IS this function's exit, so its return
        // registers already hold the value our `Return` hands over.
        if self.state.pending_sibcall.is_some() {
            return;
        }
        // Void functions have no meaningful return value — skip the store.
        // Without this, store_rax_to writes garbage %rax to a stack slot that
        // may overlap with callee-saved register save area, corrupting them.
        if return_type == IrType::Void {
            return;
        }
        if self.call_ret_is_f128_sse {
            if return_type == IrType::U16 {
                // _Float16 / __bf16 return in the low 16 bits of %xmm0.
                self.emit_scalar_xmm_to_gpr("xmm0", "rax", Some("eax"));
                self.state.emit("    movzwl %ax, %eax");
                self.store_rax_to(dest);
                self.state.reg_cache.invalidate_all();
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                return;
            }
            // _Float128 returns come back in ONE XMM register (xmm0, 16 bytes).
            // Mark the dest as a 16-byte payload value: its slot holds the
            // DATA directly (resolve_slot_addr must answer Direct, and
            // copies/loads/stores must treat it as full 128-bit).
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_reg_rbp(
                    &format!("    {}", self.xmm_move(false)),
                    "xmm0",
                    slot.0,
                );
                self.state.i128_values.insert(dest.0);
                self.state.f128_direct_slots.insert(dest.0);
            }
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        // One SSE eightbyte (`struct { float x, y; }`, `struct { double }`,
        // an 8-byte vector): the aggregate's bits come back in the low half
        // of %xmm0, while the IR carries them as a packed integer.
        if self.call_ret_classes[..] == [crate::common::types::EightbyteClass::Sse]
            && !return_type.is_float()
            && !is_i128_type(return_type)
        {
            self.store_xmm_to(dest, "xmm0", IrType::F64);
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        if is_i128_type(return_type) {
            use crate::common::types::EightbyteClass;
            // MOVQ xmm->m64 is SSE2; MOVLPS stores the same eight bytes with
            // SSE1 (`-mno-sse2 -msse`).
            let xmm_store = if self.isa.simd {
                "    movq"
            } else {
                "    movlps"
            };
            if self.call_ret_classes.len() == 2 {
                let (c0, c1) = (self.call_ret_classes[0], self.call_ret_classes[1]);
                match (c0, c1) {
                    (EightbyteClass::Integer, EightbyteClass::Sse) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                            // SSE eightbyte: store %xmm0 directly (movq
                            // xmm→m64 is baseline SSE2); the old %rdx relay
                            // was an FP-domain round-trip for no reason.
                            self.state
                                .out
                                .emit_instr_reg_rbp(xmm_store, "xmm0", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Integer) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            // Direct xmm0→slot store (baseline SSE2 movq);
                            // no GPR relay.
                            self.state.out.emit_instr_reg_rbp(xmm_store, "xmm0", slot.0);
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Sse) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            // Both halves are SSE: two direct xmm stores,
                            // replacing the double movq→rax relay (2 extra
                            // insns and two false GPR dependencies).
                            self.state.out.emit_instr_reg_rbp(xmm_store, "xmm0", slot.0);
                            self.state
                                .out
                                .emit_instr_reg_rbp(xmm_store, "xmm1", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                    }
                    _ => {
                        self.store_rax_rdx_to(dest);
                    }
                }
            } else {
                self.store_rax_rdx_to(dest);
            }
        } else if return_type == IrType::F32 || return_type == IrType::D32 {
            self.store_xmm_to(dest, "xmm0", IrType::F32);
        } else if return_type == IrType::F128 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", slot.0);
                self.state.out.emit_instr_rbp("    fldt", slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.park_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.store_rax_to(dest);
            }
        } else if return_type == IrType::F64 || return_type == IrType::D64 {
            self.store_xmm_to(dest, "xmm0", IrType::F64);
        } else {
            self.store_rax_to(dest);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_call_move_f32_to_acc_impl(&mut self) {
        self.emit_scalar_xmm_to_gpr("xmm0", "rax", Some("eax"));
    }

    pub(super) fn emit_call_move_f64_to_acc_impl(&mut self) {
        self.emit_scalar_xmm_to_gpr("xmm0", "rax", None);
    }
}

/// The unaligned move and register name that carry a `size`-byte
/// one-vector-register aggregate (psABI SSE + SSEUPs) in SSE argument /
/// return register `idx`: `movdqu %xmmN` (16), `vmovdqu %ymmN` (32, AVX),
/// `vmovdqu64 %zmmN` (64, AVX-512F). Unaligned on purpose: the object's
/// home is a stack slot or a caller buffer whose alignment the frame does
/// not promise beyond 16.
pub(super) fn vector_reg_move(size: usize, idx: usize) -> (&'static str, String) {
    match size {
        32 => ("vmovdqu", format!("ymm{idx}")),
        64 => ("vmovdqu64", format!("zmm{idx}")),
        _ => {
            // The psABI XMM carrier exists with SSE1 alone. Keep the SSE2
            // spelling on normal targets, but never introduce MOVDQU merely
            // to marshal a legal SSE1 float vector argument or return value.
            let instruction = if crate::common::x86_features::effective()
                .contains(crate::common::x86_features::X86Feature::Sse2)
            {
                "movdqu"
            } else {
                "movups"
            };
            (instruction, format!("xmm{idx}"))
        }
    }
}

#[cfg(test)]
mod indirect_fptr_parking_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    // PhysReg(11) is the x86-64 indirect-target register (%r10).
    const R10: PhysReg = PhysReg(11);

    fn emitter_with_rsp_frame() -> X86Codegen {
        let mut cg = X86Codegen::new();
        // Make rsp_frame_size moves observable alongside the emitted text.
        cg.state.out.use_rsp_addressing = true;
        cg
    }

    fn fptr() -> Operand {
        Operand::Value(Value(7))
    }

    /// Drive Phase 0 with the argument homed in %r10 so the emitter parks the
    /// callee pointer in the transient 16-byte stack slot.
    fn parked_emitter() -> X86Codegen {
        let mut cg = emitter_with_rsp_frame();
        cg.reg_assignments.insert(7, PhysReg(14)); // fptr: %rdi -> %rax
        cg.reg_assignments.insert(1, R10); // argument home: %r10
        let args = [Operand::Value(Value(1))];
        cg.emit_call_spill_fptr_with_args_impl(&fptr(), &args);
        assert!(cg.indirect_fptr_on_stack);
        cg
    }

    #[test]
    fn fast_path_preloads_r10_when_no_argument_uses_r10() {
        let mut cg = emitter_with_rsp_frame();
        cg.reg_assignments.insert(7, PhysReg(14));
        cg.reg_assignments.insert(1, PhysReg(15)); // argument in %rsi
        let args = [Operand::Value(Value(1))];
        cg.emit_call_spill_fptr_with_args_impl(&fptr(), &args);
        let out = &cg.state.out.buf;
        assert!(out.contains("movq %rax, %r10"), "{out}");
        assert!(
            !out.contains("subq $16, %rsp"),
            "fast path must not park the pointer: {out}"
        );
        assert!(!cg.indirect_fptr_on_stack);
        assert_eq!(cg.state.out.rsp_frame_size, 0);
    }

    #[test]
    fn entry_of_every_spill_resets_a_stale_parking_flag() {
        let mut cg = emitter_with_rsp_frame();
        cg.indirect_fptr_on_stack = true;
        cg.reg_assignments.insert(7, PhysReg(14));
        cg.reg_assignments.insert(1, PhysReg(15));
        let args = [Operand::Value(Value(1))];
        cg.emit_call_spill_fptr_with_args_impl(&fptr(), &args);
        assert!(
            !cg.indirect_fptr_on_stack,
            "a stale flag must not survive a new spill entry point"
        );
    }

    #[test]
    fn r10_homed_argument_parks_fptr_in_transient_stack_slot() {
        let mut cg = emitter_with_rsp_frame();
        cg.reg_assignments.insert(7, PhysReg(14));
        cg.reg_assignments.insert(1, R10);
        let args = [Operand::Value(Value(1))];
        cg.emit_call_spill_fptr_with_args_impl(&fptr(), &args);
        let out = &cg.state.out.buf;
        assert!(out.contains("subq $16, %rsp"), "{out}");
        assert!(out.contains("movq %rax, (%rsp)"), "{out}");
        assert!(
            !out.contains("movq %rax, %r10"),
            "the %r10 argument must not be clobbered: {out}"
        );
        assert!(cg.indirect_fptr_on_stack);
        assert_eq!(cg.state.out.rsp_frame_size, 16);
    }

    #[test]
    fn zero_stack_reload_reclaims_slot_and_clears_flag() {
        let mut cg = parked_emitter();
        cg.reload_spilled_indirect_fptr_to_r10(0);
        let out = &cg.state.out.buf;
        assert!(out.contains("movq (%rsp), %r10"), "{out}");
        assert!(out.contains("addq $16, %rsp"), "{out}");
        assert!(!cg.indirect_fptr_on_stack);
        assert_eq!(cg.state.out.rsp_frame_size, 0);
    }

    #[test]
    fn stack_arg_reload_keeps_slot_until_call_cleanup_reclaims() {
        let mut cg = parked_emitter();
        // 24 bytes of outgoing stack args: parity pad makes pushed_bytes 32.
        cg.reload_spilled_indirect_fptr_to_r10(24);
        let out = &cg.state.out.buf;
        assert!(out.contains("movq 32(%rsp), %r10"), "{out}");
        assert!(
            cg.indirect_fptr_on_stack,
            "the slot stays allocated across the call"
        );
        assert_eq!(cg.state.out.rsp_frame_size, 16);
        cg.emit_call_cleanup_impl(24, 0, false);
        assert!(!cg.indirect_fptr_on_stack);
        assert_eq!(cg.state.out.rsp_frame_size, 0);
        // 24 args + 8 parity pad + 16 fptr slot = 48, single combined addq.
        assert!(
            cg.state.out.buf.contains("addq $48, %rsp"),
            "{}",
            cg.state.out.buf
        );
    }

    #[test]
    fn dyn_align_reload_reads_through_save_slot_and_cleanup_reclaims() {
        let mut cg = parked_emitter();
        cg.dyn_align_cleanup = true;
        cg.reload_spilled_indirect_fptr_to_r10(64);
        let out = &cg.state.out.buf;
        // area = (64 + 15) & !15 = 64; the save slot holds the anchor whose
        // top qword is the parked pointer.
        assert!(out.contains("movq 64(%rsp), %r10"), "{out}");
        assert!(out.contains("movq (%r10), %r10"), "{out}");
        assert!(
            cg.indirect_fptr_on_stack,
            "the dyn-align cleanup path owns the reclaim"
        );
        cg.emit_call_cleanup_impl(64, 0, false);
        assert!(!cg.indirect_fptr_on_stack);
        assert_eq!(cg.state.out.rsp_frame_size, 0);
    }

    #[test]
    fn back_to_back_indirect_calls_keep_frame_accounting_balanced() {
        let mut cg = emitter_with_rsp_frame();
        cg.reg_assignments.insert(7, PhysReg(14));
        cg.reg_assignments.insert(1, R10);
        let args = [Operand::Value(Value(1))];
        for _ in 0..3 {
            cg.emit_call_spill_fptr_with_args_impl(&fptr(), &args);
            assert_eq!(cg.state.out.rsp_frame_size, 16);
            assert!(cg.indirect_fptr_on_stack);
            cg.reload_spilled_indirect_fptr_to_r10(0);
            assert_eq!(cg.state.out.rsp_frame_size, 0);
            assert!(!cg.indirect_fptr_on_stack);
        }
    }
}
