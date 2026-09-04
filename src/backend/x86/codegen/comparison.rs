//! X86Codegen: comparison and select operations.

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, X86Codegen};
use crate::backend::regalloc::PhysReg;
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrCmpOp, IrConst, Operand, Value};

/// IR-level scan for compare-replay candidates, shared by the x86-64
/// prologue (which consumes `replay` as the emitter contract) and the RA
/// link builder (which consumes `operand_links` to extend operand live
/// intervals) — allocator and emitter can never disagree about which
/// values the backend reads at the consumer position.
///
/// A Cmp is a replay candidate when its destination has exactly one use,
/// the compare is an integer compare of non-wide type, it is not already
/// handled by the better ADJACENT flag fusion (`fused`), and that single
/// use is a same-block Select or the block-terminator CondBranch. The
/// consumer then re-emits the comparison from the recorded operands right
/// before its cmov/jcc instead of testing a materialized boolean.
///
/// `operand_links` maps each Value operand to the Cmp DEST: the dest's
/// single use IS the consumer, so the dest's live-interval end marks the
/// replay position. Extending the operand's interval to that end (via
/// RegAllocConfig::folded_index_uses, whose contract — "the backend reads
/// this value at the consumer's position" — covers this case exactly)
/// guarantees register homes survive intervening instructions. Operands
/// that end up with no home at all (never-materialized values) are
/// pruned post-RA by the prologue; slot-homed operands never needed the
/// extension (the slot is written at every definition).
pub(crate) struct CmpReplayScan {
    pub replay: crate::common::fx_hash::FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>,
    pub operand_links: crate::common::fx_hash::FxHashMap<u32, Vec<u32>>,
    /// FP-Cmp records for the FP-SELECT blend (S05): float Cmp dests whose
    /// single use is a Select; keyed by Cmp dest with the recorded operands
    /// re-derived as a vcmpsd/vcmpss mask by the select.
    pub fp_select: crate::common::fx_hash::FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>,
}

pub(crate) fn compute_cmp_replay_scan(
    func: &crate::ir::reexports::IrFunction,
    use_counts: &crate::common::fx_hash::FxHashMap<u32, u32>,
    fused: &crate::common::fx_hash::FxHashMap<u32, u32>,
) -> CmpReplayScan {
    let mut replay = crate::common::fx_hash::FxHashMap::default();
    let mut operand_links: crate::common::fx_hash::FxHashMap<u32, Vec<u32>> =
        crate::common::fx_hash::FxHashMap::default();
    let mut fp_select: crate::common::fx_hash::FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)> =
        crate::common::fx_hash::FxHashMap::default();
    for block in &func.blocks {
        let insts = &block.instructions;
        for (ii, inst) in insts.iter().enumerate() {
            let (cdest, cop, clhs, crhs, cty) = match inst {
                crate::ir::reexports::Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => (dest.0, *op, lhs.clone(), rhs.clone(), *ty),
                _ => continue,
            };
            if use_counts.get(&cdest).copied().unwrap_or(0) != 1 {
                continue;
            }
            // Already handled by the (better) adjacent fusion (integer
            // Cmp→Select adjacency; FP Cmp→CondBranch adjacency). For FP
            // Cmp→Select there is NO fusion (the prologue fuse gate excludes
            // it — FP fusion is CondBranch-only), so the FP-SELECT branch
            // below remains reachable for every FP Cmp whose single use is a
            // Select.
            if fused.contains_key(&cdest) {
                continue;
            }
            // Locate the single use: a same-block Select or the
            // block-terminator CondBranch.
            let mut used_by_select = false;
            for (jj, other) in insts.iter().enumerate() {
                if jj == ii {
                    continue;
                }
                if let crate::ir::reexports::Instruction::Select {
                    cond: Operand::Value(v),
                    ..
                } = other
                {
                    if v.0 == cdest {
                        used_by_select = true;
                        break;
                    }
                }
            }
            let mut used_by_branch = false;
            if !used_by_select {
                if let crate::ir::reexports::Terminator::CondBranch {
                    cond: Operand::Value(v),
                    ..
                } = &block.terminator
                {
                    if v.0 == cdest {
                        used_by_branch = true;
                    }
                }
            }
            // FP-SELECT blend (S05): a float Cmp consumed ONLY by a Select
            // emits no boolean — the select re-derives the VEX blend mask
            // (vcmpsd/vcmpss) and blends the FP arms (vblendvpd/vblendvps).
            // A CondBranch consumer is excluded: branches keep the (better)
            // flag-fusion jcc path or the materialized boolean. The operand
            // links use the SAME consumer-position contract as the integer
            // replay above, so register homes survive to the select.
            if matches!(cty, IrType::F64 | IrType::F32) {
                if used_by_select {
                    for op in [&clhs, &crhs] {
                        if let Operand::Value(v) = op {
                            operand_links.entry(v.0).or_default().push(cdest);
                        }
                    }
                    fp_select.insert(cdest, (cop, clhs, crhs, cty));
                }
                continue;
            }
            // Integer comparisons only: float comparisons use ucomiss/
            // ucomisd with a different flag contract (PF for NaN, the
            // setnp/sete dance) — replaying them as integer cmps produces
            // wrong selects (simd_sse2_arith regression). WIDE (I128)
            // compares are excluded too: emit_cmp routes them to
            // emit_i128_cmp, which ignores cmp_replay AND could not be
            // replayed by emit_int_cmp_replay_insn (multi-instruction
            // 128-bit compare). This gate sits AFTER the FP-SELECT branch:
            // float Cmps reach the branch above first.
            if !cty.is_integer() || crate::backend::generation::is_wide_int_type(cty) {
                continue;
            }
            if used_by_select || used_by_branch {
                for op in [&clhs, &crhs] {
                    if let Operand::Value(v) = op {
                        operand_links.entry(v.0).or_default().push(cdest);
                    }
                }
                replay.insert(cdest, (cop, clhs, crhs, cty));
            }
        }
    }
    CmpReplayScan {
        replay,
        operand_links,
        fp_select,
    }
}

impl X86Codegen {
    /// Materialize the FP boolean (`setcc` chain) — used by
    /// `emit_float_cmp_impl` right after the ucomisd and by the FP-SELECT
    /// fallback at a non-adjacent position (flags were just set there).
    fn emit_fp_cmp_setcc(&mut self, dest: &Value, op: IrCmpOp) {
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        // The Eq/Ne chains WRITE %cl — a partial clobber of %rcx. The
        // secondary cache claims "rcx holds K"; a later select reuses the
        // claim as its true-arm staging register (select_operand_to_reg
        // skips the reload for a claimed constant), so the stale claim
        // would feed the CORRUPTED arm. Drop it for the chains that touch
        // %cl; seta/setae are pure %al.
        if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            self.state.reg_cache.invalidate_sec();
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    /// Re-materialize an FP comparison at a NON-Cmp position. The FP-SELECT
    /// fallback: the Cmp elided its boolean (its result feeds only Selects)
    /// but the select could not use the blend — re-emit the loads + ucomisd
    /// so the ordinary select path below is sound.
    ///
    /// RELATIONAL ops need no boolean at all: the flags just set are the
    /// condition, so `pending_fp_cmp` is handed to the select (which maps
    /// the FP jcc ja/jae directly onto its cmov) — the select then runs its
    /// register-direct or accumulator cmov path with the in-place flags and
    /// no setcc/movzbl/test. Eq/Ne cannot fold (parity-based chains), so
    /// they materialize the boolean via the setcc chain and the select
    /// tests the materialized value (the %cl clobber in that chain is
    /// handled inside emit_fp_cmp_setcc).
    pub(super) fn emit_fp_cmp_boolean(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands { (rhs, lhs) } else { (lhs, rhs) };
        self.emit_fp_operand_to_xmm(first, ty, "xmm0");
        self.emit_fp_operand_to_xmm(second, ty, "xmm1");
        if ty == IrType::F64 {
            self.state.emit("    ucomisd %xmm1, %xmm0");
        } else {
            self.state.emit("    ucomiss %xmm1, %xmm0");
        }
        match op {
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.pending_fp_cmp = Some((dest.0, "ja"));
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.pending_fp_cmp = Some((dest.0, "jae"));
            }
            IrCmpOp::Eq | IrCmpOp::Ne => {
                self.emit_fp_cmp_setcc(dest, op);
            }
        }
    }

    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // FP-SELECT blend (S05): a float Cmp whose boolean feeds ONLY Selects
        // emits nothing here — every select re-derives the VEX blend mask
        // (vcmpsd/vcmpss, see try_emit_fp_select_blend) from the recorded
        // operands and blends the FP arms. The scan excluded `fused` dests,
        // so no pending-flag handshake state is skipped with it.
        if std::env::var_os("CCC_NO_FP_SELECT").is_none() && self.fp_select_cmps.contains_key(&dest.0)
        {
            if std::env::var_os("CCC_DEBUG_FP_SELECT").is_some() {
                eprintln!("[FP-SELECT] cmp %{} emits nothing (blend at select)", dest.0);
            }
            return;
        }
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };

        // Load first operand → %xmm0 (use constant pool for FP constants)
        self.emit_fp_operand_to_xmm(first, ty, "xmm0");
        // Load second operand → %xmm1
        self.emit_fp_operand_to_xmm(second, ty, "xmm1");

        if ty == IrType::F64 {
            self.state.emit("    ucomisd %xmm1, %xmm0");
        } else {
            self.state.emit("    ucomiss %xmm1, %xmm0");
        }

        // FLAG FUSION: when this FP Cmp's boolean result is consumed ONLY by
        // an immediately-following Select/CondBranch (precomputed in
        // fused_cmp_dests), skip the boolean materialization. The ucomisd
        // above set EFLAGS; the relational ordered comparisons map cleanly
        // to `ja`/`jae` (NaN → unordered → not-taken = false, which is the
        // C99 result for NaN in any relational). Eq/Ne are never fused
        // (the prologue gate excludes them), so the parity bit is irrelevant
        // here. The consumer reads `pending_fp_cmp` for the pre-computed
        // jcc — distinct from the integer `pending_cmp` because the FP
        // flag→jcc mapping differs (ja vs jg).
        let fusion_disabled = std::env::var("CCC_NO_FLAG_FUSION").is_ok();
        if !fusion_disabled {
            if let Some(&chain_end) = self.fused_cmp_dests.get(&dest.0) {
                let jcc: Option<&'static str> = match op {
                    IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => Some("ja"),
                    IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => Some("jae"),
                    // Eq/Ne are excluded from FP fusion by the prologue gate;
                    // reaching here is unreachable, but materialize anyway
                    // for safety rather than risk a skipped boolean.
                    IrCmpOp::Eq | IrCmpOp::Ne => None,
                };
                if let Some(jcc) = jcc {
                    self.pending_fp_cmp = Some((chain_end, jcc));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }

        self.emit_fp_cmp_setcc(dest, op);
    }

    /// Consume a pending FP fused-Cmp's live flags for a boolean condition.
    /// Returns the pre-computed jcc if `cond` matches the fused Cmp's dest
    /// (or the chain-end value of a flag-neutral Copy/Cast chain), else None.
    fn take_pending_fp_cmp(&mut self, cond: &Operand) -> Option<&'static str> {
        if let Operand::Value(v) = cond {
            if let Some((dest, jcc)) = self.pending_fp_cmp {
                if dest == v.0 {
                    self.pending_fp_cmp = None;
                    return Some(jcc);
                }
            }
        }
        None
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap_x87 = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first_x87, second_x87) = if swap_x87 { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first_x87);
        self.emit_f128_load_to_x87(second_x87);
        self.state.emit("    fucomip %st(1), %st");
        self.state.emit("    fstp %st(0)");
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        // %cl partial-clobber (see emit_fp_cmp_setcc): drop the sec claim.
        if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            self.state.reg_cache.invalidate_sec();
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // COMPARE-REPLAY: the Cmp's single use is a non-adjacent Select or
        // CondBranch (recorded in cmp_replay). Skip the ENTIRE instruction —
        // including the cmp itself — because the consumer re-emits the
        // comparison from the recorded operands right before its cmov/jcc.
        // Emitting the cmp here too would double the compare in the hot loop.
        if self.cmp_replay.contains_key(&dest.0) {
            // The comparison re-emits later from the recorded operands, so
            // any deferred narrowing widens (PF-15) must materialize NOW:
            // the replay will read these homes at a distance where the
            // adjacency guarantee no longer holds.
            self.flush_pending_widen_impl();
            return;
        }
        self.emit_int_cmp_insn_typed(lhs, rhs, ty);

        // FLAG FUSION: when this Cmp's boolean result is consumed ONLY by the
        // immediately-following Select or CondBranch (precomputed in
        // fused_cmp_dests from the IR adjacency), skip the boolean
        // materialization entirely — the flags set by the cmp/test above are
        // still live, and the consumer emits jcc/cmovcc directly. This saves
        // setcc + movzbl + (movq/testq) per comparison in branch-heavy hot
        // loops (gzip's longest_match). The dispatch loop runs instructions
        // strictly in order, so the consumer is guaranteed to be the next
        // emission; if anything else intervenes (cannot happen by
        // construction), the consumer falls back to materializing the bool —
        // but that path is unreachable for fused cmps.
        //
        // SOUNDNESS: flag fusion is DISABLED when a select fallback that
        // materializes the condition via `operand_to_rax(cond)` is forced
        // (CCC_V9_SELECT = legacy v9 select emission; CCC_NO_INPLACE_SELECT =
        // disable the in-place condition test). Those legacy paths READ the
        // materialized boolean from the Cmp's register/slot, so the Cmp MUST
        // emit the setcc+movzbl — otherwise the legacy select reads a stale
        // register (it was never written because the Cmp skipped the
        // materialization). Without this gate, setting either flag produced
        // wrong select results (differential test: got 660 vs expected 1320).
        let fusion_disabled = std::env::var("CCC_NO_FLAG_FUSION").is_ok()
            || std::env::var("CCC_V9_SELECT").is_ok()
            || std::env::var("CCC_NO_INPLACE_SELECT").is_ok();
        if !fusion_disabled {
            if let Some(&chain_end) = self.fused_cmp_dests.get(&dest.0) {
                // The consumer is the next instruction(s) (possibly after a
                // flag-neutral Copy chain); record pending flags under the
                // chain-end value so the consumer's cond matches.
                self.pending_cmp = Some((chain_end, op));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        let set_instr = match op {
            IrCmpOp::Eq => "sete",
            IrCmpOp::Ne => "setne",
            IrCmpOp::Slt => "setl",
            IrCmpOp::Sle => "setle",
            IrCmpOp::Sgt => "setg",
            IrCmpOp::Sge => "setge",
            IrCmpOp::Ult => "setb",
            IrCmpOp::Ule => "setbe",
            IrCmpOp::Ugt => "seta",
            IrCmpOp::Uge => "setae",
        };
        self.state.emit_fmt(format_args!("    {} %al", set_instr));

        // Register-direct: movzbl %al, %dest_reg_32 — skip %rax relay.
        // Safe because %al is part of %rax, never overlaps callee-saved registers.
        if let Some(d_reg) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name_32(d_reg);
                self.state
                    .emit_fmt(format_args!("    movzbl %al, %{}", d_name));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        self.emit_int_cmp_insn_typed(lhs, rhs, ty);

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        self.state
            .emit_fmt(format_args!("    {} {}", jcc, true_label));
        self.state.out.emit_jmp_label(false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_fused_cmp_branch_blocks_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        self.emit_int_cmp_insn_typed(lhs, rhs, ty);

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        if self.state.next_block_label == Some(true_block) {
            self.state
                .out
                .emit_jcc_block(Self::invert_jcc(jcc), false_block.0);
        } else {
            self.state.out.emit_jcc_block(jcc, true_block.0);
            if self.state.next_block_label != Some(false_block) {
                self.state.out.emit_jmp_block(false_block.0);
            }
        }
        self.state.reg_cache.invalidate_all();
    }

    /// Fused BitTest→CondBranch: `bt index, base` then `jc/jnc`. CF holds
    /// the selected bit; nothing is materialized. The base (mask) prefers
    /// its register home; constants and slot-resident values stage through
    /// %rax (the accumulator scratch — dead here because the BitTest result
    /// was never materialized). A variable index stages through %rcx.
    pub(super) fn emit_fused_bit_test_branch_blocks_impl(
        &mut self,
        base: &Operand,
        index: &Operand,
        ty: IrType,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        let use_32bit = ty.size() <= 4;
        // Stage the BASE for the r/m operand of BT.
        let base_reg: String = match base {
            Operand::Value(v) => {
                // XMM-homed base (bit-punned float word): no GPR names;
                // stage through the accumulator, which handles xmm->GPR.
                if let Some(&reg) = self
                    .reg_assignments
                    .get(&v.0)
                    .filter(|r| !super::emit::is_xmm_reg(**r))
                {
                    if use_32bit {
                        format!("%{}", super::emit::phys_reg_name_32(reg))
                    } else {
                        format!("%{}", super::emit::phys_reg_name(reg))
                    }
                } else {
                    self.operand_to_rax(base);
                    if use_32bit {
                        "%eax".to_string()
                    } else {
                        "%rax".to_string()
                    }
                }
            }
            Operand::Const(_) => {
                self.operand_to_rax(base);
                if use_32bit {
                    "%eax".to_string()
                } else {
                    "%rax".to_string()
                }
            }
        };
        // BT with an immediate index; otherwise index in a register.
        let const_index = match index {
            Operand::Const(c) => c.to_i64().filter(|v| *v >= 0 && *v <= i32::MAX as i64),
            _ => None,
        };
        let bt = if use_32bit { "btl" } else { "btq" };
        if let Some(imm) = const_index {
            let width: u32 = if use_32bit { 32 } else { 64 };
            let bit = (imm as u32) % width;
            self.state
                .emit_fmt(format_args!("    {bt} ${bit}, {base_reg}"));
        } else {
            // The index register: prefer its home when it does not alias the
            // staged base; otherwise stage through %rcx.
            let idx_reg: String = match index {
                Operand::Value(v) => match self
                    .reg_assignments
                    .get(&v.0)
                    .filter(|r| !super::emit::is_xmm_reg(**r))
                {
                    Some(&reg) => {
                        let name = if use_32bit {
                            format!("%{}", super::emit::phys_reg_name_32(reg))
                        } else {
                            format!("%{}", super::emit::phys_reg_name(reg))
                        };
                        if name == base_reg {
                            // Same register for base and index: bt r,r is
                            // still well-defined (tests bit idx%width of the
                            // same value) — keep it.
                            name
                        } else {
                            name
                        }
                    }
                    None => {
                        self.operand_to_rcx(index);
                        if use_32bit {
                            "%ecx".to_string()
                        } else {
                            "%rcx".to_string()
                        }
                    }
                },
                _ => {
                    self.operand_to_rcx(index);
                    if use_32bit {
                        "%ecx".to_string()
                    } else {
                        "%rcx".to_string()
                    }
                }
            };
            self.state
                .emit_fmt(format_args!("    {bt} {idx_reg}, {base_reg}"));
        }
        // CF = tested bit. jc = branch when set (BitTest result nonzero).
        if self.state.next_block_label == Some(true_block) {
            self.state.out.emit_jcc_block("jnc", false_block.0);
        } else {
            self.state.out.emit_jcc_block("jc", true_block.0);
            if self.state.next_block_label != Some(false_block) {
                self.state.out.emit_jmp_block(false_block.0);
            }
        }
        self.state.reg_cache.invalidate_all();
    }

    /// Map an integer comparison opcode to the jcc mnemonic for the
    /// condition "the comparison is true".
    fn cmp_jcc(op: IrCmpOp) -> &'static str {
        match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        }
    }

    /// Map an integer comparison opcode to the cmov CONDITION CODE (the part
    /// between "cmov" and the size suffix, e.g. "g" for cmovgq). The caller
    /// composes `cmov{cc}q`.
    fn cmp_cmov(op: IrCmpOp) -> &'static str {
        match op {
            IrCmpOp::Eq => "e",
            IrCmpOp::Ne => "ne",
            IrCmpOp::Slt => "l",
            IrCmpOp::Sle => "le",
            IrCmpOp::Sgt => "g",
            IrCmpOp::Sge => "ge",
            IrCmpOp::Ult => "b",
            IrCmpOp::Ule => "be",
            IrCmpOp::Ugt => "a",
            IrCmpOp::Uge => "ae",
        }
    }

    /// If `cond` is the destination of a fused Cmp whose flags are still
    /// pending, consume the pending flags and return the comparison opcode.
    fn take_pending_cmp(&mut self, cond: &Operand) -> Option<IrCmpOp> {
        if let Operand::Value(v) = cond {
            if let Some((dest, op)) = self.pending_cmp {
                if dest == v.0 {
                    self.pending_cmp = None;
                    return Some(op);
                }
            }
        }
        None
    }

    /// If `cond` is the destination of a Cmp eligible for compare-replay,
    /// consume the replay record (op + operands) so the consumer can re-emit
    /// the comparison and use cmovcc/jcc directly.
    fn take_replay_cmp(&mut self, cond: &Operand) -> Option<(IrCmpOp, Operand, Operand, IrType)> {
        if let Operand::Value(v) = cond {
            if let Some(rec) = self.cmp_replay.remove(&v.0) {
                return Some(rec);
            }
        }
        None
    }

    #[inline]
    fn invert_jcc(cc: &str) -> &'static str {
        match cc {
            "je" => "jne",
            "jne" => "je",
            "jl" => "jge",
            "jge" => "jl",
            "jle" => "jg",
            "jg" => "jle",
            "jb" => "jae",
            "jae" => "jb",
            "jbe" => "ja",
            "ja" => "jbe",
            _ => "jne",
        }
    }

    pub(super) fn emit_cond_branch_blocks_impl(
        &mut self,
        cond: &Operand,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        let next = self.state.next_block_label;
        // Profile-driven fallthrough. The layout pass recorded which
        // successor carries more profile executions (the hot edge); the
        // codegen driver sets it before this call. We make that successor the
        // physical fallthrough (emit a conditional jump to the COLD successor
        // instead), so the hot path takes no branch — Intel branch-prediction
        // guidance — without reordering blocks and thus without perturbing
        // register allocation. When no hint is present (or it is ambiguous),
        // we default to preferring the TRUE successor, preserving the original
        // behavior.
        let pref_true = crate::pgo::take_cond_fallthrough()
            .map(|h| h == true_block.0)
            .unwrap_or(true);
        let (hot, cold) = if pref_true {
            (true_block, false_block)
        } else {
            (false_block, true_block)
        };
        let hot_next = next == Some(hot);
        let cold_next = next == Some(cold);

        // FLAG FUSION (FP): the condition is the direct result of an
        // immediately-preceding float Cmp whose ucomisd flags are still
        // live. Branch on the pre-computed FP jcc (ja/jae family) directly,
        // skipping the setcc + movzbl + testq chain — the mandelbrot
        // `if (zr*zr + zi*zi > 4.0) break` drops from 6 instructions
        // (ucomisd; seta; movzbl; movq; test; jne) to 2 (ucomisd; ja).
        if let Some(jcc) = self.take_pending_fp_cmp(cond) {
            let jcc_hot = if pref_true {
                jcc
            } else {
                Self::invert_jcc(jcc)
            };
            if hot_next {
                self.state
                    .out
                    .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
            } else {
                self.state.out.emit_jcc_block(jcc_hot, hot.0);
                if !cold_next {
                    self.state.out.emit_jmp_block(cold.0);
                }
            }
            self.state.reg_cache.invalidate_all();
            return;
        }

        // FLAG FUSION: the condition is the direct result of the immediately
        // preceding Cmp; its flags are live — branch on them directly,
        // skipping the testq of the materialized boolean.
        if let Some(op) = self.take_pending_cmp(cond) {
            let jcc = Self::cmp_jcc(op);
            let jcc_hot = if pref_true {
                jcc
            } else {
                Self::invert_jcc(jcc)
            };
            if hot_next {
                self.state
                    .out
                    .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
            } else {
                self.state.out.emit_jcc_block(jcc_hot, hot.0);
                if !cold_next {
                    self.state.out.emit_jmp_block(cold.0);
                }
            }
            self.state.reg_cache.invalidate_all();
            return;
        }

        // COMPARE-REPLAY: non-adjacent Cmp consumed by this branch. Re-emit
        // the comparison (fresh flags) and branch on the condition code
        // directly — kills the setcc/movzbl + testq chain per branch.
        if let Some((op, lhs, rhs, ty)) = self.take_replay_cmp(cond) {
            self.state.reg_cache.invalidate_acc();
            self.emit_int_cmp_replay_insn(&lhs, &rhs, ty);
            let jcc = Self::cmp_jcc(op);
            let jcc_hot = if pref_true {
                jcc
            } else {
                Self::invert_jcc(jcc)
            };
            if hot_next {
                self.state
                    .out
                    .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
            } else {
                self.state.out.emit_jcc_block(jcc_hot, hot.0);
                if !cold_next {
                    self.state.out.emit_jmp_block(cold.0);
                }
            }
            self.state.reg_cache.invalidate_all();
            return;
        }

        // Register-direct: test the condition register directly, skip %rax relay.
        // WIDE conditions never take this arm: their homes are the 16-byte
        // slot or the rax:rdx pair (never a single GPR), and a single-half
        // test is unsound — they route to the two-half form below.
        if let Operand::Value(v) = cond {
            if !self.cond_is_wide(v.0) {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if !is_xmm_reg(reg) {
                        self.emit_cond_gpr_test(v.0, reg);
                        let jcc_hot = if pref_true { "jne" } else { "je" };
                        if hot_next {
                            self.state
                                .out
                                .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
                        } else {
                            self.state.out.emit_jcc_block(jcc_hot, hot.0);
                            if !cold_next {
                                self.state.out.emit_jmp_block(cold.0);
                            }
                        }
                        return;
                    }
                }
            }
        }
        // WIDE conditions: two half-width tests (lo, hi) — a wide value is
        // zero iff BOTH halves are zero. A single 64-bit test of either half
        // misclassifies `(__int128)1 << 64` (low half 0, high half 1) as
        // zero and branches to the wrong arm.
        if matches!(cond, Operand::Value(v) if self.cond_is_wide(v.0)) {
            self.emit_wide_cond_branch(cond, hot, cold, hot_next, cold_next, pref_true);
            return;
        }
        self.operand_to_rax(cond);
        self.state.emit("    testq %rax, %rax");
        let jcc_hot = if pref_true { "jne" } else { "je" };
        if hot_next {
            self.state
                .out
                .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
        } else {
            self.state.out.emit_jcc_block(jcc_hot, hot.0);
            if !cold_next {
                self.state.out.emit_jmp_block(cold.0);
            }
        }
    }

    /// Emit `test{size} %reg, %reg` for a GPR-homed condition value, sized to
    /// the value's IR type.
    ///
    /// The historical form was always `testq %r64, %r64`. That is wrong for
    /// every condition narrower than 64 bits: the SysV x86-64 ABI leaves the
    /// bits above a parameter's width UNDEFINED, so `testq` on a 32-bit
    /// condition reads garbage that may set/clear ZF regardless of the actual
    /// value — an `if (c)` / `c ? a : b` on a zero `int` could spuriously
    /// branch/select the true arm when the caller passed stale high bits.
    /// Size the test to the value's own width (`testl`/`testw`/`testb`),
    /// exactly as the stack-slot path already sizes its memory compare
    /// (`emit_cmp_zero_mem` → `cmpl` for I32 homed bools).
    fn emit_cond_gpr_test(&mut self, val_id: u32, reg: PhysReg) {
        // Only integer (and pointer) conditions carry a zero/nonzero
        // semantic worth testing; bools arrive as I8. For every other
        // recorded type keep the historical full-width test.
        if let Some(ty) = self.value_types.get(&val_id).copied() {
            match ty {
                IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => {
                    let (_, test_mnem, _) = super::emit::cmp_width_info(ty);
                    let name = super::emit::typed_phys_reg_name(reg, ty);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, %{}", test_mnem, name, name));
                    return;
                }
                _ => {}
            }
        }
        let name = phys_reg_name(reg);
        self.state
            .emit_fmt(format_args!("    testq %{}, %{}", name, name));
    }

    /// Test the select condition in place, WITHOUT materializing it into a
    /// register and WITHOUT `pushfq`/`popfq`:
    ///
    ///   * register-allocated cond → `test{size} %reg, %reg` (no clobbers;
    ///                               sized to the condition's IR width so a
    ///                               32-bit condition never reads the
    ///                               undefined upper register bits)
    ///   * stack-slot cond         → `cmpl $0, off(%rsp/%rbp)` (no clobbers;
    ///                               bools are stored as zero-extended I32)
    ///   * accumulator-cached cond → `testq %rax, %rax`
    ///
    /// Returns true when the flags were set in place (so the caller can emit a
    /// cmov directly). `pushfq`/`popfq` are serializing (~20-50 cycles each) and
    /// additionally disable the store-forwarding peephole phases, so avoiding
    /// them in the common case is a large hot-loop win.
    fn test_select_cond_in_place(&mut self, cond: &Operand) -> bool {
        if let Operand::Value(v) = cond {
            if !self.state.is_alloca(v.0) {
                // WIDE (128-bit) conditions: zero-ness is (lo | hi) == 0, so a
                // single 64-bit test on either half is unsound — a value with
                // a zero low half and a nonzero high half is NOT zero (this
                // is the `(__int128)1 << 64` class). Route to the wide
                // in-place form BEFORE any single-register path can claim the
                // condition: a wide value's homes are the 16-byte slot or the
                // rax:rdx accumulator pair, never a single GPR.
                if self.cond_is_wide(v.0) {
                    return self.test_wide_cond_in_place(v.0);
                }
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if !is_xmm_reg(reg) {
                        self.emit_cond_gpr_test(v.0, reg);
                        return true;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    // Size the memory compare to the value's recorded IR
                    // type. The historical unconditional `cmpl` under-read
                    // every slot wider than 4 bytes: an I64-homed condition
                    // whose low 4 bytes are zero is NOT zero (1 << 32), yet
                    // `cmpl` reported zero. Reading fewer bytes than the
                    // store defined is always sound (the low bytes of a
                    // stored value are always defined), so the recorded type
                    // width is the safe choice; an untracked type keeps the
                    // historical form.
                    let mnem = self
                        .value_types
                        .get(&v.0)
                        .map(|ty| super::emit::cmp_width_info(*ty).0)
                        .unwrap_or("cmpl");
                    self.state.out.emit_cmp_zero_mem_sized(slot.0, mnem);
                    return true;
                }
                if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    self.state.emit("    testq %rax, %rax");
                    return true;
                }
            }
        }
        false
    }

    /// True when `val_id`'s recorded IR type is a WIDE integer (I128/U128):
    /// a value whose zero-ness spans two 64-bit halves, so any single-width
    /// zero test (`testq`, `cmpq`, `cmpl`) on one half is unsound.
    fn cond_is_wide(&self, val_id: u32) -> bool {
        self.value_types
            .get(&val_id)
            .map(|ty| crate::backend::generation::is_wide_int_type(*ty))
            .unwrap_or(false)
    }

    /// In-place zero test for a WIDE condition, SELECT context (flags must
    /// survive into a following cmovcc, so no clobber of the accumulator and
    /// no branch). Zero-ness is `(lo | hi) == 0`; `orq` sets ZF from exactly
    /// that.
    ///
    ///   * slot home      → `movq slot(%rbp), %rcx; orq slot+8(%rbp), %rcx`
    ///   * acc-pair home  → `movq %rax, %rcx; orq %rdx, %rcx`
    ///     (rax = lo, rdx = hi — the i128 accumulator-pair invariant)
    ///
    /// Only %rcx is written, and the SELECT flow stages its arms into %rcx
    /// strictly AFTER the test, so the clobber is consumed before any live
    /// use. The secondary-register cache is invalidated to keep the
    /// `select_operand_to_reg` discipline (a stale "rcx holds K" claim must
    /// not survive a register it no longer describes).
    ///
    /// Returns false when the condition's home is neither a known slot nor
    /// the tracked accumulator pair (the caller falls back to the generic
    /// materialize-and-test path).
    fn test_wide_cond_in_place(&mut self, val_id: u32) -> bool {
        if let Some(slot) = self.state.get_slot(val_id) {
            // lo at slot+0, hi at slot+8 (the i128 slot layout used by
            // emit_store_pair_to_slot / the Mov128 path).
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
            self.state
                .out
                .emit_instr_rbp_reg("    orq", slot.0 + 8, "rcx");
            self.state.reg_cache.invalidate_sec();
            return true;
        }
        if self.state.reg_cache.acc_has(val_id, false) {
            self.state.emit("    movq %rax, %rcx");
            self.state.emit("    orq %rdx, %rcx");
            self.state.reg_cache.invalidate_sec();
            return true;
        }
        false
    }

    /// Branch-context zero test for a WIDE condition: fold the two halves
    /// into the accumulator and let ONE conditional branch decide — the
    /// GCC shape (`movq lo, %rax; orq hi, %rax; jcc`). Zero-ness of a wide
    /// value is `(lo | hi) == 0`, so the fold is exact; two independent
    /// half-tests would each need the OTHER half's verdict to place a jump
    /// (a zero low half does NOT imply a zero value), which is exactly the
    /// trap this fold avoids.
    ///
    ///   * slot home     → `movq slot(%rbp), %rax; orq slot+8(%rbp), %rax`
    ///   * acc-pair home → `orq %rdx, %rax` (rax already holds lo)
    ///   * anything else → stage the pair with `operand_to_rax_rdx` (handles
    ///     I128 constants, indirect homes, alloca addressing), then fold
    ///
    /// Clobbering rax (and rdx when staging) at a block-terminating branch
    /// matches the generic fallback's contract — the accumulator is staging
    /// territory here and the cache is invalidated on exit.
    fn emit_wide_cond_branch(
        &mut self,
        cond: &Operand,
        hot: BlockId,
        cold: BlockId,
        hot_next: bool,
        cold_next: bool,
        pref_true: bool,
    ) {
        let val_id = match cond {
            Operand::Value(v) => v.0,
            Operand::Const(_) => u32::MAX, // constants: stage via operand_to_rax_rdx
        };
        let slot = match cond {
            Operand::Value(v) => self.state.get_slot(v.0),
            Operand::Const(_) => None,
        };
        if let Some(slot) = slot {
            // lo at slot+0, hi at slot+8 (the i128 slot layout used by
            // emit_store_pair_to_slot / the Mov128 path).
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
            self.state
                .out
                .emit_instr_rbp_reg("    orq", slot.0 + 8, "rax");
        } else if self.state.reg_cache.acc_has(val_id, false) {
            // rax = lo, rdx = hi (the i128 accumulator-pair invariant).
            self.state.emit("    orq %rdx, %rax");
        } else {
            self.operand_to_rax_rdx(cond);
            self.state.emit("    orq %rdx, %rax");
        }
        let jcc_hot = if pref_true { "jne" } else { "je" };
        if hot_next {
            self.state
                .out
                .emit_jcc_block(Self::invert_jcc(jcc_hot), cold.0);
        } else {
            self.state.out.emit_jcc_block(jcc_hot, hot.0);
            if !cold_next {
                self.state.out.emit_jmp_block(cold.0);
            }
        }
        self.state.reg_cache.invalidate_all();
    }

    /// Legacy-path condition test (pushfq/popfq sites): stage the condition
    /// and set ZF from its zero-ness. WIDE conditions stage the full rax:rdx
    /// pair and fold the halves with `orq` — a wide value is zero iff BOTH
    /// halves are zero, so a single-half `testq` misclassifies
    /// `(__int128)1 << 64`. Everything else keeps the historical
    /// single-register form. The callers save/restore flags around their arm
    /// staging (pushfq/popfq), so the fold's register mutation is safe.
    fn emit_legacy_cond_test(&mut self, cond: &Operand) {
        let wide = matches!(cond, Operand::Value(v) if self.cond_is_wide(v.0));
        if wide {
            self.operand_to_rax_rdx(cond);
            self.state.emit("    orq %rdx, %rax");
        } else {
            self.operand_to_rax(cond);
            self.state.emit("    testq %rax, %rax");
        }
    }

    /// Materialize a select operand into a register using ONLY flag-neutral
    /// instructions. The generic `operand_to_rcx`/`operand_to_callee_reg`
    /// materialize zero constants as `xorl %reg, %reg`, which clobbers the
    /// condition flags — unacceptable between the in-place condition test and
    /// the cmov (that is exactly why the legacy path needed pushfq/popfq).
    /// Value operands are always flag-neutral in the existing helpers
    /// (mov/movl/movslq/leaq family), so only constants are handled here.
    fn select_operand_to_reg(&mut self, op: &Operand, reg64: &str, reg32: &str) {
        if let Operand::Const(c) = op {
            // The raw immediate emitters below bypass the register cache
            // bookkeeping that `operand_to_rcx`/`operand_to_rax` maintain.
            // Writing %rcx/%rax without invalidating the corresponding
            // cache entry left a STALE "rcx holds K" claim alive: the next
            // `operand_to_rcx(K)` then skipped the reload and the consumer
            // read the clobbered register (`(h^v)*K` after a
            // `movq $0,%rcx; cmoveq` select returned 0-multiplied garbage —
            // bitops_builtins). The copy-propagation peephole had been
            // masking this by rewriting the consumers off %rcx; any RA
            // perturbation that broke that rescue exposed the miscompile.
            if reg64 == "rcx" {
                self.state.reg_cache.invalidate_sec();
            } else if reg64 == "rax" {
                self.state.reg_cache.invalidate_acc();
            }
            if c.is_zero() {
                // PF-16: movl $0 writes the same canonical value (zero-extend)
                // two bytes shorter; neither form touches flags.
                self.state.emit_fmt(format_args!(
                    "    movl $0, %{}",
                    super::emit::reg_name_to_32(&reg64)
                ));
                return;
            }
            match c {
                IrConst::I8(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I16(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I64(v) => {
                    if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                        self.state.out.emit_instr_imm_reg("    movq", *v, reg64);
                    } else {
                        self.state.out.emit_instr_imm_reg("    movabsq", *v, reg64);
                    }
                }
                IrConst::F32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", v.to_bits() as i64, reg64);
                }
                IrConst::F64(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", v.to_bits() as i64, reg64);
                }
                IrConst::LongDouble(v, _) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", v.to_bits() as i64, reg64);
                }
                // Decimal FP bit containers: opaque bit patterns.
                IrConst::D32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::D64(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", *v as i64, reg64);
                }
                IrConst::I128(v) => {
                    let low = *v as i64;
                    if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                        self.state.out.emit_instr_imm_reg("    movq", low, reg64);
                    } else {
                        self.state.out.emit_instr_imm_reg("    movabsq", low, reg64);
                    }
                }
                IrConst::Zero => {
                    // PF-16: movl $0 writes the same canonical value (zero-extend)
                    // two bytes shorter; neither form touches flags.
                    self.state.emit_fmt(format_args!(
                        "    movl $0, %{}",
                        super::emit::reg_name_to_32(&reg64)
                    ));
                }
            }
            let _ = reg32;
            return;
        }
        if reg64 == "rcx" {
            self.operand_to_rcx(op);
        } else if reg64 == "rax" {
            self.operand_to_rax(op);
        } else {
            // Register-direct destination: find the PhysReg by name.
            let phys = match reg64 {
                "rbx" => Some(PhysReg(1)),
                "r12" => Some(PhysReg(2)),
                "r13" => Some(PhysReg(3)),
                "r14" => Some(PhysReg(4)),
                "r15" => Some(PhysReg(5)),
                "rbp" => Some(PhysReg(6)),
                "r11" => Some(PhysReg(10)),
                "r10" => Some(PhysReg(11)),
                "r8" => Some(PhysReg(12)),
                "r9" => Some(PhysReg(13)),
                "rdi" => Some(PhysReg(14)),
                "rsi" => Some(PhysReg(15)),
                "rdx" => Some(PhysReg(16)),
                _ => None,
            };
            if let Some(p) = phys {
                self.operand_to_callee_reg(op, p);
            } else {
                self.operand_to_rax(op);
            }
        }
    }

    /// `cmovl`/`cmovq` chosen by type. 32-bit cmov is restricted to unsigned
    /// types: `cmovl` zero-extends dest, which is the U32 home invariant but
    /// would drop the sign of a negative I32 that a later 64-bit signed use
    /// (without an explicit `movslq`) would read.
    fn emit_cmov_typed(&mut self, cc: &str, src64: &str, dst64: &str, ty: IrType) {
        if ty.size() <= 4 {
            // 32-bit cmov: only the low 32 bits of each operand are selected
            // and written, which is exactly the semantics of an I32/U32 select
            // regardless of signedness. (64-bit cmovs on 32-bit data would
            // additionally require every arm to be sign-extended to 64 bits.)
            self.state.emit_fmt(format_args!(
                "    cmov{}l %{}, %{}",
                cc,
                super::emit::reg_name_to_32(src64),
                super::emit::reg_name_to_32(dst64),
            ));
        } else {
            self.state
                .emit_fmt(format_args!("    cmov{}q %{}, %{}", cc, src64, dst64));
        }
    }

    /// S05 FP-SELECT: blend the FP arms of a Select whose condition is a
    /// float Cmp that elided its boolean (see emit_float_cmp_impl). The mask
    /// is re-derived as vcmpsd/vcmpss from the recorded operands, then
    /// vblendvpd/vblendvps selects the arms — gcc's exact shape.
    ///
    /// Register contract: XMM homes are xmm2-xmm15, so xmm0/xmm1 are the
    /// only scratch XMMs and no operand can alias them. Both arms are pure
    /// reads with the compare-replay consumer-position liveness contract
    /// (the operand links extended their intervals to this select).
    ///
    /// Blend order is TRUE-ARM-SECOND: vblendvpd %mask, %true, %false, %dst
    /// (the VEX /is4 src2 — the AT&T middle operand — is selected when the
    /// mask MSB is 1). Reversing it silently swaps the arms — the
    /// `rint(-0.5) → -0.5` miscompile class.
    ///
    /// Returns true when the blend was emitted; false when the operands /
    /// destination cannot be blended in registers (the caller then
    /// re-materializes the boolean and uses the ordinary path).
    fn try_emit_fp_select_blend(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) -> bool {
        // vcmpsd predicate per the ucomisd-setcc semantics:
        //   * Eq  → EQ_OQ (0):  ordered equal, NaN → false  (setnp&sete)
        //   * Ne  → NEQ_UQ (4): unordered OR not-equal, NaN → true (setp|setne)
        //   * Slt → LT_OQ (17), Sle → LE_OQ (18), Sgt → GT_OQ (14),
        //     Sge → GE_OQ (13): ordered, NaN → false (seta/setae).
        // The OQ (quiet) variants avoid signaling-NaN exceptions while
        // preserving bit-exact predicate results. NOTE the RAW (lhs, rhs)
        // order — vcmpsd encodes the predicate directly, unlike the
        // swapped ucomisd+seta dance.
        let imm: u8 = match op {
            IrCmpOp::Eq => 0,
            IrCmpOp::Ne => 4,
            IrCmpOp::Slt | IrCmpOp::Ult => 17,
            IrCmpOp::Sle | IrCmpOp::Ule => 18,
            IrCmpOp::Sgt | IrCmpOp::Ugt => 14,
            IrCmpOp::Sge | IrCmpOp::Uge => 13,
        };
        let is_f64 = ty == IrType::F64;
        let cmp_insn = if is_f64 { "vcmpsd" } else { "vcmpss" };
        let blend_insn = if is_f64 { "vblendvpd" } else { "vblendvps" };

        // Case A: XMM-homed destination — blend straight into the home.
        if let Some(d) = self.dest_reg(dest).filter(|r| is_xmm_reg(*r)) {
            let d_name = phys_reg_name(d);
            if std::env::var_os("CCC_DEBUG_FP_SELECT").is_some() {
                eprintln!("[FP-SELECT] blend into {}", d_name);
            }
            // 1. mask: lhs → xmm0, rhs → xmm1, mask → xmm0. (The operands'
            //    homes can only be xmm2-xmm15, so no aliasing with scratch.)
            self.emit_fp_operand_to_xmm(lhs, ty, "xmm0");
            self.emit_fp_operand_to_xmm(rhs, ty, "xmm1");
            self.state
                .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0, %xmm0", cmp_insn, imm));
            // 2. true → xmm1 BEFORE false can clobber a shared home.
            self.emit_fp_operand_to_xmm(true_val, ty, "xmm1");
            // 3. false → dest home (no-op when the false arm already lives
            //    there; the true-arm copy above stays safe either way).
            self.emit_fp_operand_to_xmm(false_val, ty, d_name);
            // 4. blend: mask ? true : false, into the dest home.
            self.state.emit_fmt(format_args!(
                "    {} %xmm0, %xmm1, %{}, %{}",
                blend_insn, d_name, d_name
            ));
            self.state.reg_cache.invalidate_acc();
            return true;
        }

        // Case B: destination has no XMM home — blend into the xmm0 FP
        // accumulator, then store_xmm0_fp_dest (XMM home is impossible here
        // by construction; GPR home / slot / accumulator all handled).
        // Requires the FALSE arm to be XMM-homed: only src1 (vvvv) can take
        // a spare register in this budget — mask in xmm1, true in xmm0, so
        // the false arm is read straight from its home register.
        let false_home = match false_val {
            Operand::Value(v) => self
                .reg_assignments
                .get(&v.0)
                .copied()
                .filter(|r| is_xmm_reg(*r)),
            _ => None,
        };
        let Some(f_reg) = false_home else {
            return false;
        };
        let f_name = phys_reg_name(f_reg);
        if std::env::var_os("CCC_DEBUG_FP_SELECT").is_some() {
            eprintln!("[FP-SELECT] blend into xmm0 (false in {})", f_name);
        }
        // 1. mask → xmm1: lhs → xmm0, rhs → xmm1, vcmpsd into xmm1.
        self.emit_fp_operand_to_xmm(lhs, ty, "xmm0");
        self.emit_fp_operand_to_xmm(rhs, ty, "xmm1");
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0, %xmm1", cmp_insn, imm));
        // 2. true → xmm0 (any operand kind; the false arm stays in its home).
        self.emit_fp_operand_to_xmm(true_val, ty, "xmm0");
        // 3. blend into the accumulator: mask ? xmm0 : %f.
        self.state.emit_fmt(format_args!(
            "    {} %xmm1, %xmm0, %{}, %xmm0",
            blend_insn, f_name
        ));
        self.store_xmm0_fp_dest(dest, ty);
        return true;
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) {
        // legacy-compat path (debugging): the legacy emission order.
        if std::env::var("CCC_V9_SELECT").is_ok() {
            if let Some(d_reg) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
                if !is_xmm_reg(d_reg) {
                    let d_name = phys_reg_name(d_reg);
                    self.emit_legacy_cond_test(cond);
                    self.state.emit("    pushfq");
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 8;
                    }
                    self.operand_to_callee_reg(false_val, d_reg);
                    self.operand_to_rcx(true_val);
                    self.state.emit("    popfq");
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size -= 8;
                    }
                    self.state
                        .emit_fmt(format_args!("    cmovneq %rcx, %{}", d_name));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
            self.emit_legacy_cond_test(cond);
            self.state.emit("    pushfq");
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 8;
            }
            self.operand_to_rax(false_val);
            self.operand_to_rcx(true_val);
            self.state.emit("    popfq");
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= 8;
            }
            self.state.emit("    cmovneq %rcx, %rax");
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Constant condition: statically select the operand. Both operands are
        // pure SSA values (already computed by earlier instructions), so only
        // the chosen one needs to be loaded — no test/cmov/branch at all.
        if let Operand::Const(c) = cond {
            let chosen = if c.is_zero() { false_val } else { true_val };
            if let Some(d_reg) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
                if !is_xmm_reg(d_reg) {
                    self.operand_to_callee_reg(chosen, d_reg);
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
            self.operand_to_rax(chosen);
            self.store_rax_to(dest);
            return;
        }

        // FP-SELECT blend (S05): the condition is a float Cmp whose single
        // use is this select; the Cmp emitted no boolean (see
        // emit_float_cmp_impl). Re-derive the vcmpsd/vcmpss mask and blend
        // the FP arms. On failure (or non-blendable arms), re-materialize
        // the boolean with emit_fp_cmp_boolean and fall through to the
        // ordinary materialized-select path below.
        //
        // Arm acceptance: the blend is a pure BIT select. The frontend
        // lowers FP selects as INTEGER selects over bitcast arms (the FP
        // bits live in the same XMM homes), so the arms are accepted when
        // they carry a clean full-lane bit pattern at the Cmp's width:
        //   * cty F64 → arms typed F64 or I64
        //   * cty F32 → arms typed F32 or I32
        // (I32 values in XMM homes occupy only the low 32 bits — writing a
        // GPR→XMM lane leaves the upper bits stale — so an F64-width blend
        // over an I32 arm would mix stale lanes; the width must match.)
        if std::env::var_os("CCC_NO_FP_SELECT").is_none() {
            if let Operand::Value(cv) = cond {
                if let Some((cop, clhs, crhs, cty)) = self.fp_select_cmps.get(&cv.0).cloned() {
                    let arm_ok = |op: &Operand| -> bool {
                        match op {
                            Operand::Const(_) => true,
                            Operand::Value(v) => match self.value_types.get(&v.0).copied() {
                                Some(tt) => {
                                    tt == cty
                                        || (cty == IrType::F64 && tt == IrType::I64)
                                        || (cty == IrType::F32 && tt == IrType::I32)
                                }
                                None => false,
                            },
                        }
                    };
                    if arm_ok(true_val) && arm_ok(false_val) {
                        if !self.try_emit_fp_select_blend(
                            dest, cop, &clhs, &crhs, true_val, false_val, cty,
                        ) {
                            if std::env::var_os("CCC_DEBUG_FP_SELECT").is_some() {
                                eprintln!(
                                    "[FP-SELECT] cmp %{}: blend refused, materialize boolean",
                                    cv.0
                                );
                            }
                            self.emit_fp_cmp_boolean(cv, cop, &clhs, &crhs, cty);
                        } else {
                            return;
                        }
                    } else {
                        if std::env::var_os("CCC_DEBUG_FP_SELECT").is_some() {
                            eprintln!(
                                "[FP-SELECT] cmp %{}: arm types not blendable, materialize",
                                cv.0
                            );
                        }
                        self.emit_fp_cmp_boolean(cv, cop, &clhs, &crhs, cty);
                    }
                }
            }
        }

        // FLAG FUSION: the condition is the direct result of the immediately
        // preceding Cmp whose flags are live — use the comparison's condition
        // code directly for the cmov, no test needed.
        let fused_op = self.take_pending_cmp(cond);
        // FP-SELECT fallback flags: emit_fp_cmp_boolean (above) just ran the
        // ucomisd and left its FP relational flags live. The FP flag → cmov
        // mapping is ja/jae (unordered → not-taken → false arm, C99 NaN), the
        // SAME contract as the CondBranch fusion.
        let fused_fp = self.take_pending_fp_cmp(cond);
        // COMPARE-REPLAY: the condition is a Cmp result consumed only by this
        // select but not adjacent to the Cmp (flags clobbered in between).
        // Re-emit the comparison from the recorded operands right before the
        // cmov; the flags are then fresh. The accumulator cache is invalidated
        // first so re-materialized operands are always reloaded from their
        // canonical locations.
        let replay_op = self.take_replay_cmp(cond);
        if let Some((op, lhs, rhs, ty)) = &replay_op {
            self.state.reg_cache.invalidate_acc();
            self.emit_int_cmp_replay_insn(lhs, rhs, *ty);
        }
        // Test the condition in place (no pushfq). Only when the condition is
        // not directly testable (rare) do we fall back to the legacy
        // materialize + pushfq/popfq path. With live FP flags the in-place
        // path is MANDATORY (the condition's boolean was never materialized,
        // so any load/test would read a stale home).
        let tested_in_place = if std::env::var("CCC_NO_INPLACE_SELECT").is_ok() {
            false
        } else if fused_op.is_some() || fused_fp.is_some() || replay_op.is_some() {
            true
        } else {
            self.test_select_cond_in_place(cond)
        };
        let cmov_cc = match (fused_op, fused_fp, replay_op.as_ref()) {
            (Some(op), _, _) => Self::cmp_cmov(op),
            (None, Some("ja"), _) => "a",
            (None, Some("jae"), _) => "ae",
            (None, Some(_), _) => "ne",
            (None, None, Some((op, _, _, _))) => Self::cmp_cmov(*op),
            (None, None, None) => "ne",
        };

        // Register-direct: when dest has a register, operate directly on it.
        if let Some(d_reg) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                if tested_in_place {
                    // Load order (all alias-safe):
                    //   1. true_val → %rcx   (clobbers rcx only; rcx is never
                    //      register-allocated, and this only READS other regs)
                    //   2. false_val → dest  (clobbers dest; cond was already
                    //      tested, so flags are safe; if true_val lived in dest,
                    //      rcx already holds a copy)
                    //   3. cmovcc — reads flags (set by the in-place test or the
                    //      fused Cmp; not modified by any of the above moves,
                    //      which are flag-neutral) and rcx.
                    // All materialization is flag-neutral (select_operand_to_reg
                    // avoids `xorl` for zero constants, which would clobber the
                    // flags between the test and the cmov).
                    //
                    // Direct-cmov: when the true value already lives in a
                    // (non-XMM, non-dest) register, emit `cmovcc %src, %dest`
                    // straight from it — skipping the dead `movq %src, %rcx`
                    // staging copy (fill_window's prev[] loop had one per
                    // element). The rcx staging is only needed when true_val
                    // aliases dest or isn't register-resident.
                    let true_src_reg = match true_val {
                        Operand::Value(v) => self
                            .reg_assignments
                            .get(&v.0)
                            .copied()
                            .filter(|r| !is_xmm_reg(*r) && *r != d_reg),
                        _ => None,
                    };
                    if let Some(src_reg) = true_src_reg {
                        self.select_operand_to_reg(
                            false_val,
                            phys_reg_name(d_reg),
                            phys_reg_name_32(d_reg),
                        );
                        self.emit_cmov_typed(cmov_cc, phys_reg_name(src_reg), d_name, ty);
                    } else {
                        self.select_operand_to_reg(true_val, "rcx", "ecx");
                        self.select_operand_to_reg(
                            false_val,
                            phys_reg_name(d_reg),
                            phys_reg_name_32(d_reg),
                        );
                        self.emit_cmov_typed(cmov_cc, "rcx", d_name, ty);
                    }
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                // Legacy path: condition not testable in place.
                self.emit_legacy_cond_test(cond);
                self.state.emit("    pushfq");
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size += 8;
                }
                self.operand_to_callee_reg(false_val, d_reg);
                self.operand_to_rcx(true_val);
                self.state.emit("    popfq");
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size -= 8;
                }
                self.state
                    .emit_fmt(format_args!("    cmovneq %rcx, %{}", d_name));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        // Accumulator fallback.
        if tested_in_place {
            // false_val → %rax, true_val → %rcx: both loads only clobber
            // rax/rcx and never modify the flags set by the in-place test or
            // the fused Cmp (flag-neutral materialization, see
            // select_operand_to_reg).
            self.select_operand_to_reg(false_val, "rax", "eax");
            self.select_operand_to_reg(true_val, "rcx", "ecx");
            self.emit_cmov_typed(cmov_cc, "rcx", "rax", ty);
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Legacy pushfq fallback (condition not testable in place).
        self.emit_legacy_cond_test(cond);
        self.state.emit("    pushfq");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += 8;
        }
        self.operand_to_rax(false_val);
        self.operand_to_rcx(true_val);
        self.state.emit("    popfq");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= 8;
        }
        self.state.emit("    cmovneq %rcx, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }
}
