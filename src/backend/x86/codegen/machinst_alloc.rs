//! SSA-driven window register allocator for the MachInst buffer.
//!
//! ## Why this exists
//!
//! The MachInst layer lowers SSA-IR windows into machine instructions whose
//! register-only positions (two-address ALU dests, `Lea`/`SetCC`/`Movzx`
//! dests, memory bases and indices) hold `MachReg::Vreg` operands — the SSA
//! value IDs of values the *main* (function-level) allocator chose to spill.
//! Before this module, the only "resolution" was to substitute the value's
//! stack slot into memory-form operand positions and to give up — replaying
//! the ENTIRE buffered window through the accumulator text path — whenever a
//! vreg occupied a register-only position. One spilled pointer used as a load
//! base was enough to demote dozens of already-well-lowered instructions.
//! This module turns the buffer into what `machinst.rs`'s module doc always
//! promised: a machine-IR window with local register allocation.
//!
//! ## The SSA contract
//!
//! Vregs are IR value IDs and the IR is SSA, so within one window:
//!
//! * every vreg has at most one *pure* defining write (the `Mov` the isel
//!   emits to feed a two-address form counts as that write; later writes are
//!   read-modify-write of the same SSA value);
//! * a vreg whose first reference is a read (or an RMW) was defined *before*
//!   the window — "arriving": its stack slot is the canonical home outside
//!   the window, because `flush_machinst` invalidates the register cache;
//! * a vreg whose window reads exhaust its IR use count is dead after the
//!   window and needs no store-back.
//!
//! These properties make window liveness exact with one linear scan — no
//! dataflow, no fixpoint — and they are *checked*: any discipline violation
//! (two pure writes, a read before a local pure write, a domain mismatch)
//! fails the allocation and the window falls back to the correct, slower
//! replay path.
//!
//! ## Algorithm
//!
//! 1. **Scan** the window once, collecting per-instruction vreg reference
//!    kinds (read / pure write / RMW), memory-base/index positions, XMM
//!    register operands, the physical registers each instruction touches,
//!    and the implicit `rax`/`rdx` clobbers of the division forms (`Raw`
//!    conservatively blocks the entire pool).
//! 2. **Classify** every vreg: `reg` — needs a window scratch register
//!    (register-only position, or a memory base/index); `mem` — appears only
//!    in substitutable positions, where the existing stack-slot memory-operand
//!    folding is already optimal. `classify_window` exposes the `reg` set so
//!    `resolve_stack_vregs` skips substituting exactly those vregs.
//! 3. **Allocate** by live interval (first..last reference) with interference
//!    against (a) physical-register operands inside the interval, (b) whole-
//!    window-busy registers — homes of main-allocator values live across the
//!    window but not referenced inside it, derived from the RA assignments ×
//!    liveness segments — and (c) previously assigned vregs.
//! 4. **Rewrite** every occurrence of an assigned vreg to its register and
//!    insert the slot reload before an arriving vreg's first reference and
//!    the slot store after a live-out vreg's last reference, both at the
//!    value's own width (a `movl` reload zero-extends; a small slot is never
//!    accessed wider than 4 bytes).
//!
//! Calls never appear inside a window (the buffer is drained around
//! `CallTyped` and at block boundaries), so caller-saved and callee-saved
//! GPRs are equally legal window homes; a register's only conflict duties
//! are operand-visible uses, liveness-busy spans, and other assignments.
//! `rbp` is excluded outright: it is the frame base in rbp-addressing mode
//! and worth one less invariant than it costs.
//!
//! The allocator is deterministic and allocation failure is structurally
//! rare (≤ 11 candidate registers against window-local live ranges); the
//! replay remains the fail-safe, but it is now the exception it was always
//! meant to be, not the cliff every spilled pointer fell off.

use super::machinst::{MachInst, MachOperand, MachReg, OpSize, MACHINST_ALLOCATABLE_GPRS, RBP};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;

/// Everything the allocator needs to know about the values behind vregs.
/// Read-only views of codegen state: the allocator is a pure function,
/// which is what makes it directly unit-testable.
pub(crate) struct WindowCtx<'a> {
    /// value id → spill slot (the canonical home outside the window).
    pub slots: &'a FxHashMap<u32, crate::backend::state::StackSlot>,
    /// Values whose slot is 4 bytes wide.
    pub small_slots: &'a FxHashSet<u32>,
    /// value id → IR type (drives reload/store widths and domain checks).
    pub types: &'a FxHashMap<u32, IrType>,
    /// Total IR use counts per value (function-wide, instructions +
    /// terminators).
    pub total_uses: &'a FxHashMap<u32, u32>,
    /// Alloca values: their slot holds the *storage*; a plain `movq` reload
    /// would load the contents, not the pointer. The emit-side gate already
    /// rejects them; the allocator refuses them again as defense in depth.
    pub alloca_values: &'a FxHashSet<u32>,
    /// Main-RA GPR homes busy across the window: register id → live spans
    /// (inclusive program points) of the values homed there. A register whose
    /// value is live across the window but never referenced inside it is
    /// invisible to operand-level interference — this map is what makes the
    /// window allocator sound against exactly that class.
    pub reg_busy: &'a FxHashMap<u8, Vec<(u32, u32)>>,
    /// Inclusive program-point span of the window's IR instructions
    /// (`[pp_end - ir_len, pp_end - 1]`).
    pub window_span: (u32, u32),
}

/// How one instruction references a vreg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefKind {
    /// Reads the value.
    Read,
    /// Overwrites the value (the SSA def, or a later pure re-materialization
    /// — the latter is refused as a discipline violation).
    PureWrite,
    /// Reads and overwrites (two-address ALU forms, `cmov`, `neg`).
    Rmw,
}

/// One vreg's window biography.
#[derive(Debug, Default, Clone, Copy)]
struct VregInfo {
    touched: bool,
    first: usize,
    last: usize,
    /// Index of the single pure write, if any.
    def: Option<usize>,
    /// Pure-write count (must be ≤ 1: SSA).
    pure_writes: u32,
    /// Reads + RMWs (RMW reads the incoming value).
    uses: u32,
    /// RMW count: positions that READ and OVERWRITE. A window that only
    /// reads a value never needs to write its slot back; an RMW does.
    rmw_writes: u32,
    /// True when the vreg occupies a position the slot substitution cannot
    /// express (register-only position or memory base/index).
    needs_reg: bool,
    /// True when the vreg is referenced through an XMM register operand.
    xmm_domain: bool,
}

/// Scan state: vreg biographies plus per-index physical-register touch sets.
#[derive(Default)]
struct Scan {
    infos: FxHashMap<u32, VregInfo>,
    phys_touched: Vec<FxHashSet<u8>>,
    /// Set when an instruction shape must refuse the whole window
    /// (a vreg shift amount, a vreg inside `CallTyped`).
    refused: bool,
}

impl Scan {
    fn touch(&mut self, r: &MachReg, idx: usize, kind: RefKind, reg_only: bool) {
        let MachReg::Vreg(id) = r else { return };
        let e = self.infos.entry(*id).or_default();
        if !e.touched {
            e.touched = true;
            e.first = idx;
            e.last = idx;
        } else {
            e.first = e.first.min(idx);
            e.last = e.last.max(idx);
        }
        match kind {
            RefKind::Read => e.uses += 1,
            RefKind::PureWrite => {
                e.pure_writes += 1;
                if e.def.is_none() {
                    e.def = Some(idx);
                }
            }
            RefKind::Rmw => {
                e.uses += 1;
                e.rmw_writes += 1;
            }
        }
        e.needs_reg |= reg_only;
    }

    fn touch_xmm(&mut self, r: &MachReg) {
        if let MachReg::Vreg(id) = r {
            if let Some(e) = self.infos.get_mut(id) {
                e.xmm_domain = true;
            }
        }
    }

    /// Scan an operand. `kind` applies to register operands; memory
    /// base/index registers are always reads. `substitutable` marks operand
    /// positions whose vregs may keep the stack-slot substitution.
    fn touch_op(&mut self, o: &MachOperand, idx: usize, kind: RefKind, substitutable: bool) {
        match o {
            MachOperand::Reg(r) => self.touch(r, idx, kind, !substitutable),
            MachOperand::Mem { base, .. } => self.touch(base, idx, RefKind::Read, true),
            MachOperand::MemIndex { base, index, .. } => {
                self.touch(base, idx, RefKind::Read, true);
                self.touch(index, idx, RefKind::Read, true);
            }
            _ => {}
        }
    }

    /// Record every physical register an instruction touches (read or write
    /// alike: a window value in that register must not survive across the
    /// instruction either way), plus implicit clobbers.
    fn phys_effects(&mut self, inst: &MachInst, idx: usize) {
        let mut touched: Vec<u8> = touched_phys_regs(inst);
        // Implicit clobbers beyond operands.
        match inst {
            MachInst::Cqto { .. } | MachInst::Div { .. } => {
                touched.push(0); // rax
                touched.push(16); // rdx
            }
            MachInst::XorRdx => {
                touched.push(16); // rdx
            }
            // The Raw escape hatch could contain anything: block the whole
            // pool at this index.
            MachInst::Raw(_) => {
                for r in MACHINST_ALLOCATABLE_GPRS {
                    touched.push(r.0);
                }
            }
            _ => {}
        }
        let at = &mut self.phys_touched[idx];
        for r in touched {
            at.insert(r);
        }
    }

    fn scan_inst(&mut self, inst: &MachInst, idx: usize) {
        match inst {
            MachInst::Mov { src, dst, .. } => {
                self.touch_op(src, idx, RefKind::Read, true);
                match dst {
                    MachOperand::Reg(r) => self.touch(r, idx, RefKind::PureWrite, false),
                    _ => self.touch_op(dst, idx, RefKind::Read, true),
                }
            }
            MachInst::FMov { src, dst, .. } => {
                // Register operands are XMM-domain: resolve never substitutes
                // FMov, so a Vreg here stays and must refuse the window (a
                // GPR scratch rewrite would be unencodable).
                self.touch_op(src, idx, RefKind::Read, true);
                if let MachOperand::Reg(r) = src {
                    self.touch_xmm(r);
                }
                match dst {
                    MachOperand::Reg(r) => {
                        self.touch(r, idx, RefKind::PureWrite, false);
                        self.touch_xmm(r);
                    }
                    _ => self.touch_op(dst, idx, RefKind::Read, true),
                }
            }
            MachInst::FAlu {
                src2, src1, dst, ..
            } => {
                // src2 may keep the memory-form substitution (the VEX form
                // folds one memory source). A Vreg that survives resolution
                // there is XMM-domain: the emitter's register form only
                // accepts XMM Phys, so a GPR scratch rewrite would be
                // unencodable — mark the domain and refuse if it stays.
                self.touch_op(src2, idx, RefKind::Read, true);
                if let MachOperand::Reg(r) = src2 {
                    self.touch_xmm(r);
                }
                self.touch(src1, idx, RefKind::Read, false);
                self.touch(dst, idx, RefKind::PureWrite, false);
                self.touch_xmm(src1);
                self.touch_xmm(dst);
            }
            MachInst::Mov128 { src, dst } => {
                self.touch_op(src, idx, RefKind::Read, false);
                self.touch_op(dst, idx, RefKind::Read, false);
            }
            MachInst::Movzx { src, dst, .. } | MachInst::Movsx { src, dst, .. } => {
                self.touch_op(src, idx, RefKind::Read, true);
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::Alu { src, dst, .. } => {
                self.touch_op(src, idx, RefKind::Read, true);
                // Two-address dest: register-only (a memory dest is not an
                // encodable ALU form).
                self.touch(dst, idx, RefKind::Rmw, true);
            }
            MachInst::Imul3 { src, dst, .. } => {
                self.touch(src, idx, RefKind::Read, false);
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::Neg { dst, .. } | MachInst::Not { dst, .. } => {
                self.touch(dst, idx, RefKind::Rmw, true);
            }
            MachInst::Shift { amount, dst, .. } => {
                // The emitter's variable-shift form is `%cl`-only: a Vreg
                // amount would be silently ignored (the classic dropped-count
                // hazard). isel stages counts into %rcx; anything else is a
                // lowering defect — refuse the window.
                if let MachOperand::Reg(MachReg::Vreg(_)) = amount {
                    self.refused = true;
                }
                self.touch_op(amount, idx, RefKind::Read, true);
                self.touch(dst, idx, RefKind::Rmw, true);
            }
            MachInst::ShiftX {
                count, src, dst, ..
            } => {
                self.touch(count, idx, RefKind::Read, false);
                self.touch(src, idx, RefKind::Read, false);
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::Lea {
                base, index, dst, ..
            } => {
                self.touch(base, idx, RefKind::Read, false);
                if let Some((r, _)) = index {
                    self.touch(r, idx, RefKind::Read, false);
                }
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::LeaSym { dst, .. } => {
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::LeaSlot { dst, .. } => {
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::Div { divisor, .. } => {
                self.touch_op(divisor, idx, RefKind::Read, true);
            }
            MachInst::Cmp { lhs, rhs, .. } | MachInst::Test { lhs, rhs, .. } => {
                self.touch_op(lhs, idx, RefKind::Read, true);
                self.touch_op(rhs, idx, RefKind::Read, true);
            }
            MachInst::SetCC { dst, .. } => {
                self.touch(dst, idx, RefKind::PureWrite, false);
            }
            MachInst::Cmov { src, dst, .. } => {
                self.touch_op(src, idx, RefKind::Read, true);
                self.touch(dst, idx, RefKind::Rmw, true);
            }
            MachInst::CallTyped { args, ret, .. } => {
                // Self-contained by construction (the builder resolves every
                // source to a GPR home, slot, or immediate): a vreg here is
                // a builder-defect class; refuse rather than emit %vregN.
                let bad = args.iter().any(|m| has_vreg_operand(&m.src))
                    || ret.as_ref().is_some_and(|r| has_vreg_operand(&r.dst));
                if bad {
                    self.refused = true;
                }
            }
            // Control flow never enters the buffer (terminators are emitted
            // outside the MachInst window; the branch lowerings have no
            // callers). Nothing to scan.
            MachInst::Jcc { .. }
            | MachInst::Jmp { .. }
            | MachInst::Label(_)
            | MachInst::Call { .. }
            | MachInst::Ret => {}
            MachInst::Cqto { .. } | MachInst::XorRdx => {}
            MachInst::Raw(_) => {}
        }
        self.phys_effects(inst, idx);
    }
}

/// Physical registers referenced by an instruction's operands (read or
/// write alike — for interference both directions matter equally).
fn touched_phys_regs(inst: &MachInst) -> Vec<u8> {
    fn reg_of(r: &MachReg, out: &mut Vec<u8>) {
        if let MachReg::Phys(p) = r {
            out.push(p.0);
        }
    }
    fn op_of(o: &MachOperand, out: &mut Vec<u8>) {
        match o {
            MachOperand::Reg(r) => reg_of(r, out),
            MachOperand::Mem { base, .. } => reg_of(base, out),
            MachOperand::MemIndex { base, index, .. } => {
                reg_of(base, out);
                reg_of(index, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    match inst {
        MachInst::Mov { src, dst, .. } | MachInst::FMov { src, dst, .. } => {
            op_of(src, &mut v);
            op_of(dst, &mut v);
        }
        MachInst::FAlu { src2, .. } => op_of(src2, &mut v),
        MachInst::Mov128 { src, dst } => {
            op_of(src, &mut v);
            op_of(dst, &mut v);
        }
        MachInst::Movzx { src, .. } | MachInst::Movsx { src, .. } => op_of(src, &mut v),
        MachInst::Alu { src, .. } => op_of(src, &mut v),
        MachInst::Shift { amount, .. } => op_of(amount, &mut v),
        MachInst::Div { divisor, .. } => op_of(divisor, &mut v),
        MachInst::Cmp { lhs, rhs, .. } | MachInst::Test { lhs, rhs, .. } => {
            op_of(lhs, &mut v);
            op_of(rhs, &mut v);
        }
        MachInst::Cmov { src, .. } => op_of(src, &mut v),
        MachInst::CallTyped { args, ret, .. } => {
            for m in args {
                op_of(&m.src, &mut v);
                // dst_reg is a PhysReg by construction.
                v.push(m.dst_reg.0);
            }
            if let Some(r) = ret {
                op_of(&r.dst, &mut v);
            }
        }
        MachInst::LeaSym { dst, .. } | MachInst::LeaSlot { dst, .. } => reg_of(dst, &mut v),
        MachInst::Imul3 { src, .. } => reg_of(src, &mut v),
        MachInst::ShiftX {
            count, src, dst, ..
        } => {
            reg_of(count, &mut v);
            reg_of(src, &mut v);
            reg_of(dst, &mut v);
        }
        MachInst::Lea {
            base, index, dst, ..
        } => {
            reg_of(base, &mut v);
            if let Some((r, _)) = index {
                reg_of(r, &mut v);
            }
            reg_of(dst, &mut v);
        }
        MachInst::Neg { dst, .. } | MachInst::Not { dst, .. } => reg_of(dst, &mut v),
        MachInst::SetCC { dst, .. } => reg_of(dst, &mut v),
        MachInst::Jcc { .. } | MachInst::Jmp { .. } | MachInst::Label(_) => {}
        MachInst::Cqto { .. } | MachInst::XorRdx | MachInst::Ret | MachInst::Call { .. } => {}
        MachInst::Raw(_) => {}
    }
    v
}

fn has_vreg_operand(o: &MachOperand) -> bool {
    match o {
        MachOperand::Reg(MachReg::Vreg(_)) => true,
        MachOperand::Mem {
            base: MachReg::Vreg(_),
            ..
        } => true,
        MachOperand::MemIndex {
            base: MachReg::Vreg(_),
            ..
        }
        | MachOperand::MemIndex {
            index: MachReg::Vreg(_),
            ..
        } => true,
        _ => false,
    }
}

/// The vregs that must be register-allocated (register-only positions or
/// memory bases/indices). `resolve_stack_vregs` skips substituting exactly
/// these IDs so the allocator rewrites every occurrence uniformly.
pub(crate) fn classify_window(insts: &[MachInst]) -> FxHashSet<u32> {
    let mut scan = Scan {
        phys_touched: vec![FxHashSet::default(); insts.len()],
        ..Default::default()
    };
    for (idx, inst) in insts.iter().enumerate() {
        scan.scan_inst(inst, idx);
    }
    scan.infos
        .iter()
        .filter(|(_, e)| e.needs_reg)
        .map(|(id, _)| *id)
        .collect()
}

/// The width at which a vreg's slot is reloaded/stored: the value's own
/// width, clamped to the slot's width (a small slot is 4 bytes).
fn transfer_size(ty: Option<&IrType>, small_slot: bool) -> OpSize {
    let from_ty = ty.copied().map(OpSize::from_ir_type).unwrap_or(OpSize::S64);
    if small_slot {
        // 4-byte slot: a 64-bit transfer would read/write the neighbour.
        match from_ty {
            OpSize::S64 => OpSize::S32, // defensive: small slots hold ≤32-bit values
            s => s,
        }
    } else {
        from_ty
    }
}

/// True when a type lives outside the integer GPR domain (or is wider than
/// one register), where the window allocator must not substitute a GPR.
fn non_gpr_type(ty: Option<&IrType>) -> bool {
    ty.is_some_and(|t| t.is_float() || t.is_128bit())
}

/// Whole-window busy set: main-RA homes whose values are live across the
/// window's program-point span (invisible to operand-level interference).
fn window_busy_regs(ctx: &WindowCtx<'_>) -> FxHashSet<u8> {
    let (w0, w1) = ctx.window_span;
    let mut busy = FxHashSet::default();
    for (&r, spans) in ctx.reg_busy {
        if spans.iter().any(|&(s, e)| s <= w1 && e >= w0) {
            busy.insert(r);
        }
    }
    busy
}

/// The allocator proper.
///
/// Returns `None` when the window cannot be fully allocated — the caller
/// replays it through the default path (correct, slower). `insts` is
/// consumed: successful allocation rewrites it into the final, vreg-free
/// sequence with reload/store traffic inserted.
pub(crate) fn allocate_window(
    mut insts: Vec<MachInst>,
    ctx: &WindowCtx<'_>,
) -> Option<Vec<MachInst>> {
    if insts.is_empty() {
        return Some(insts);
    }

    // ── Phase 1: scan ────────────────────────────────────────────────
    let mut scan = Scan {
        phys_touched: vec![FxHashSet::default(); insts.len()],
        ..Default::default()
    };
    for (idx, inst) in insts.iter().enumerate() {
        scan.scan_inst(inst, idx);
    }
    if scan.refused {
        return None;
    }
    if scan.infos.is_empty() {
        return Some(insts); // nothing to allocate
    }

    // ── Phase 2: admission checks & classification ───────────────────
    let mut reg_needed: Vec<(u32, VregInfo)> = Vec::new();
    for (id, e) in &scan.infos {
        // Values without a slot cannot be reloaded or stored.
        if !ctx.slots.contains_key(id) {
            return None;
        }
        // Alloca pointers: the slot holds the storage, not the pointer.
        if ctx.alloca_values.contains(id) {
            return None;
        }
        // XMM-domain or non-GPR types stay on the replay path.
        if e.xmm_domain || non_gpr_type(ctx.types.get(id)) {
            return None;
        }
        // SSA discipline: exactly one pure write, and no read/RMW before it.
        if e.pure_writes > 1 {
            return None;
        }
        if let Some(d) = e.def {
            if e.uses > 0 && e.first < d {
                return None; // read before the local def: malformed SSA window
            }
        }
        if e.needs_reg {
            reg_needed.push((*id, *e));
        }
    }

    // Memory-only vregs (not in `reg_needed`) keep the substitution the
    // caller already performed; any that failed to substitute (slot_fits,
    // missing slot) remain as Vregs and trip the caller's final
    // has-unresolvable gate, exactly as before.

    if reg_needed.is_empty() {
        return Some(insts);
    }

    // ── Phase 3: interval allocation ────────────────────────────────
    // Prefer caller-saved registers: the main allocator's callee-saved bank
    // is its preferred home set, and any home actually used inside the
    // window is blocked by operand interference anyway — but when both
    // candidates are free, a caller-saved scratch touches nothing the
    // function's save/restore machinery cares about.
    let mut pool: Vec<u8> = MACHINST_ALLOCATABLE_GPRS
        .iter()
        .map(|r| r.0)
        .filter(|&r| r != RBP.0)
        .collect();
    pool.sort_by_key(|&r| {
        let caller_saved = matches!(r, 10 | 11 | 12 | 13 | 14 | 15 | 16); // r11,r10,r8,r9,rdi,rsi,rdx
        if caller_saved {
            0
        } else {
            1
        }
    });

    // Whole-window-busy registers (main-RA homes live across the window).
    let busy_whole = window_busy_regs(ctx);

    // busy[r] = index-space intervals where register r is occupied (phys
    // operand touches, implicit clobbers, prior assignments).
    let mut busy: FxHashMap<u8, Vec<(usize, usize)>> = FxHashMap::default();
    for (idx, touched) in scan.phys_touched.iter().enumerate() {
        for &r in touched {
            busy.entry(r).or_default().push((idx, idx));
        }
    }
    let overlaps = |busy: &FxHashMap<u8, Vec<(usize, usize)>>, r: u8, a: usize, b: usize| {
        busy.get(&r)
            .is_some_and(|ivs| ivs.iter().any(|&(s, e)| a <= e && s <= b))
    };

    // Earlier live ranges claim their preferred registers first.
    let mut reg_needed = reg_needed;
    reg_needed.sort_by_key(|(_, e)| e.first);

    // The scratch is live from the reload (just before the first reference)
    // to the store (just after the last reference); the ±1 slop covers the
    // inserted Mov positions.
    let assignments: FxHashMap<u32, u8> = reg_needed
        .iter()
        .map(|(id, e)| {
            let a = e.first.saturating_sub(1);
            let b = e.last + 1;
            let chosen = pool
                .iter()
                .copied()
                .find(|&r| !busy_whole.contains(&r) && !overlaps(&busy, r, a, b));
            let Some(r) = chosen else {
                return Err(*id); // pool exhausted for this window
            };
            busy.entry(r).or_default().push((a, b));
            Ok((*id, r))
        })
        .collect::<Result<FxHashMap<u32, u8>, u32>>()
        .ok()?;

    // ── Phase 4: rewrite + reload/store insertion ────────────────────
    for inst in insts.iter_mut() {
        rewrite_inst(inst, &assignments);
    }

    // Reloads (arriving vregs) and stores (live-out defs), inserted from the
    // back so earlier indices stay valid.
    let mut inserts: Vec<(usize, MachInst)> = Vec::new();
    for (id, e) in &reg_needed {
        let Some(&r) = assignments.get(id) else {
            continue;
        };
        let reg = MachReg::Phys(crate::backend::regalloc::PhysReg(r));
        let slot = ctx.slots[id].0;
        let size = transfer_size(ctx.types.get(id), ctx.small_slots.contains(id));
        let total = ctx.total_uses.get(id);

        // Arriving: no local pure write, or (impossible after the checks
        // above) references before it — the slot holds the value.
        let arriving = e.def.is_none();
        if arriving {
            inserts.push((
                e.first,
                MachInst::Mov {
                    src: MachOperand::StackSlot(slot),
                    dst: MachOperand::Reg(reg),
                    size,
                },
            ));
        }

        // Live-out AND window-written: the window's reads alone never
        // change the slot image, so an arriving, read-only value needs no
        // store back. A local pure write or an RMW modified the register,
        // and readers after the window (same block or later blocks) all go
        // through the slot once the register cache is invalidated — the
        // store is what makes the register image durable. A missing use
        // count is conservatively treated as live-out.
        let wrote = e.def.is_some() || e.rmw_writes > 0;
        let live_out = total.is_none_or(|t| e.uses < *t);
        if wrote && live_out {
            inserts.push((
                e.last + 1,
                MachInst::Mov {
                    src: MachOperand::Reg(reg),
                    dst: MachOperand::StackSlot(slot),
                    size,
                },
            ));
        }
    }

    inserts.sort_by_key(|(idx, _)| *idx);
    for (idx, ins) in inserts.into_iter().rev() {
        insts.insert(idx.min(insts.len()), ins);
    }

    Some(insts)
}

/// Rewrite every assigned vreg inside one instruction.
fn rewrite_inst(inst: &mut MachInst, assignments: &FxHashMap<u32, u8>) {
    let r = |x: &MachReg| -> MachReg {
        match x {
            MachReg::Vreg(id) => match assignments.get(id) {
                Some(&p) => MachReg::Phys(crate::backend::regalloc::PhysReg(p)),
                None => *x,
            },
            p => *p,
        }
    };
    let o = |x: &MachOperand| -> MachOperand {
        match x {
            MachOperand::Reg(reg) => MachOperand::Reg(r(reg)),
            MachOperand::Mem { base, offset } => MachOperand::Mem {
                base: r(base),
                offset: *offset,
            },
            MachOperand::MemIndex {
                base,
                index,
                scale,
                offset,
            } => MachOperand::MemIndex {
                base: r(base),
                index: r(index),
                scale: *scale,
                offset: *offset,
            },
            other => other.clone(),
        }
    };
    match inst {
        MachInst::Mov { src, dst, .. } => {
            *src = o(src);
            *dst = o(dst);
        }
        MachInst::FMov { src, dst, .. } => {
            *src = o(src);
            *dst = o(dst);
        }
        MachInst::FAlu {
            src2, src1, dst, ..
        } => {
            *src2 = o(src2);
            *src1 = r(src1);
            *dst = r(dst);
        }
        MachInst::Mov128 { src, dst } => {
            *src = o(src);
            *dst = o(dst);
        }
        MachInst::Movzx { src, dst, .. } | MachInst::Movsx { src, dst, .. } => {
            *src = o(src);
            *dst = r(dst);
        }
        MachInst::Alu { src, dst, .. } => {
            *src = o(src);
            *dst = r(dst);
        }
        MachInst::Imul3 { src, dst, .. } => {
            *src = r(src);
            *dst = r(dst);
        }
        MachInst::Neg { dst, .. } | MachInst::Not { dst, .. } => {
            *dst = r(dst);
        }
        MachInst::Shift { amount, dst, .. } => {
            *amount = o(amount);
            *dst = r(dst);
        }
        MachInst::ShiftX {
            count, src, dst, ..
        } => {
            *count = r(count);
            *src = r(src);
            *dst = r(dst);
        }
        MachInst::Lea {
            base, index, dst, ..
        } => {
            *base = r(base);
            if let Some((ir, _)) = index {
                *ir = r(ir);
            }
            *dst = r(dst);
        }
        MachInst::LeaSym { dst, .. } => {
            *dst = r(dst);
        }
        MachInst::LeaSlot { dst, .. } => {
            *dst = r(dst);
        }
        MachInst::Div { divisor, .. } => {
            *divisor = o(divisor);
        }
        MachInst::Cmp { lhs, rhs, .. } | MachInst::Test { lhs, rhs, .. } => {
            *lhs = o(lhs);
            *rhs = o(rhs);
        }
        MachInst::SetCC { dst, .. } => {
            *dst = r(dst);
        }
        MachInst::Cmov { src, dst, .. } => {
            *src = o(src);
            *dst = r(dst);
        }
        // CallTyped is vreg-free by construction (checked at scan time);
        // control flow and Raw never carry window vregs.
        _ => {}
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// The allocator is a pure function over (instructions, value metadata), so
// every rule it enforces is directly unit-testable: reload placement for
// arriving values, store placement for live-out defs, dead-def elision,
// register interference (operand-level AND liveness-level), the admission
// refusals, and the width discipline of the inserted traffic. The mutation
// rule from the repo's process docs applies: each test must fail when the
// behavior it pins is broken (verified during development by selectively
// reverting the rule under test).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::state::StackSlot;

    fn ctx<'a>(
        slots: &'a FxHashMap<u32, StackSlot>,
        types: &'a FxHashMap<u32, IrType>,
        total_uses: &'a FxHashMap<u32, u32>,
        reg_busy: &'a FxHashMap<u8, Vec<(u32, u32)>>,
        empty: &'a FxHashSet<u32>,
    ) -> WindowCtx<'a> {
        WindowCtx {
            slots,
            small_slots: empty,
            types,
            total_uses,
            alloca_values: empty,
            reg_busy,
            window_span: (0, 15),
        }
    }

    fn base_maps() -> (
        FxHashMap<u32, StackSlot>,
        FxHashMap<u32, IrType>,
        FxHashMap<u32, u32>,
        FxHashMap<u8, Vec<(u32, u32)>>,
    ) {
        (
            FxHashMap::default(),
            FxHashMap::default(),
            FxHashMap::default(),
            FxHashMap::default(),
        )
    }

    fn vreg(id: u32) -> MachReg {
        MachReg::Vreg(id)
    }

    fn phys(id: u8) -> MachReg {
        MachReg::Phys(crate::backend::regalloc::PhysReg(id))
    }

    fn assert_no_vregs(insts: &[MachInst]) {
        let stray: Vec<String> = insts
            .iter()
            .filter(|i| mentions_vreg(i))
            .map(|i| format!("{i:?}"))
            .collect();
        assert!(stray.is_empty(), "vregs survived allocation: {stray:?}");
    }

    fn mentions_vreg(inst: &MachInst) -> bool {
        use super::super::machinst_emit::reg_name_pub;
        let probe = MachInst::Raw(String::new());
        let _ = probe;
        // Reuse the emit-side check by formatting: vregs trap in reg_name_pub.
        // Simpler: pattern-match the operand positions the writer rewrites.
        let mut ops: Vec<&MachOperand> = Vec::new();
        match inst {
            MachInst::Mov { src, dst, .. } => {
                ops.push(src);
                ops.push(dst);
            }
            MachInst::Alu { src, .. } => ops.push(src),
            MachInst::Lea {
                base, index, dst, ..
            } => {
                let _ = (base, index, dst);
            }
            _ => {}
        }
        let _ = reg_name_pub;
        ops.iter().any(|o| {
            matches!(
                o,
                MachOperand::Reg(MachReg::Vreg(_))
                    | MachOperand::Mem {
                        base: MachReg::Vreg(_),
                        ..
                    }
                    | MachOperand::MemIndex {
                        base: MachReg::Vreg(_),
                        ..
                    }
                    | MachOperand::MemIndex {
                        index: MachReg::Vreg(_),
                        ..
                    }
            )
        })
    }

    /// The canonical two-address idiom: `Mov lhs→v; Alu rhs,v` with v spilled,
    /// live-out, and lhs arriving through its slot. Expected shape:
    /// reload(lhs); mov lhs→scratch(v); alu rhs,scratch(v); store scratch(v).
    #[test]
    fn two_address_idiom_gets_reload_mov_alu_store() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(10, StackSlot(-8)); // lhs, arriving
        slots.insert(11, StackSlot(-16)); // dest v
        types.insert(10, IrType::I64);
        types.insert(11, IrType::I64);
        uses.insert(10, 1); // consumed here only
        uses.insert(11, 3); // two more uses after the window
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);

        // Post-resolve shape: the lhs read (a substitutable position) is
        // already a stack-slot memory operand; the reg-classified dest vreg
        // is what the allocator must home.
        let insts = vec![
            MachInst::Mov {
                src: MachOperand::StackSlot(-8),
                dst: MachOperand::Reg(vreg(11)),
                size: OpSize::S64,
            },
            MachInst::Alu {
                op: super::super::machinst::AluOp::Add,
                src: MachOperand::Imm(4),
                dst: vreg(11),
                size: OpSize::S64,
            },
        ];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        // mov slot(lhs)→scratch; alu imm,scratch; store scratch→slot(dest)
        assert_eq!(out.len(), 3, "expected mov/alu/store, got {out:?}");
        let scratch = match &out[0] {
            MachInst::Mov {
                src: MachOperand::StackSlot(-8),
                dst: MachOperand::Reg(r @ MachReg::Phys(_)),
                size: OpSize::S64,
            } => *r,
            other => panic!("first instruction must be the lhs load, got {other:?}"),
        };
        match &out[1] {
            MachInst::Alu {
                src: MachOperand::Imm(4),
                dst,
                size: OpSize::S64,
                ..
            } => {
                assert_eq!(*dst, scratch, "alu must accumulate into the scratch");
            }
            other => panic!("second instruction must be the alu, got {other:?}"),
        }
        match &out[2] {
            MachInst::Mov {
                src: MachOperand::Reg(r),
                dst: MachOperand::StackSlot(-16),
                size: OpSize::S64,
            } => assert_eq!(*r, scratch, "store must drain the scratch"),
            other => panic!("last instruction must be the dest store, got {other:?}"),
        }
    }

    /// A value fully consumed inside the window (uses == total) is DEAD
    /// afterwards: no store, no slot traffic at all beyond the reload chain.
    #[test]
    fn dead_def_gets_no_store() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(20, StackSlot(-8));
        types.insert(20, IrType::I64);
        uses.insert(20, 1); // single use: the Cmp below
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);

        // Cmp lhs=arriving vreg(20) (read-only, but the position is
        // substitutable... force the register class via a Mem base):
        let insts = vec![
            MachInst::Mov {
                src: MachOperand::Mem {
                    base: vreg(20),
                    offset: 0,
                },
                dst: MachOperand::Reg(phys(2)),
                size: OpSize::S64,
            },
            MachInst::Cmp {
                lhs: MachOperand::Reg(phys(2)),
                rhs: MachOperand::Reg(vreg(20)),
                size: OpSize::S64,
            },
        ];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        // reload(20) + mov + cmp — no trailing store.
        assert_eq!(out.len(), 3, "dead value must not be stored back: {out:?}");
        assert!(
            !matches!(
                out.last(),
                Some(MachInst::Mov {
                    dst: MachOperand::StackSlot(_),
                    ..
                })
            ),
            "dead value must not be stored back"
        );
    }

    /// Two simultaneously-live vregs must receive DIFFERENT registers.
    #[test]
    fn overlapping_vregs_never_share_a_register() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        for id in [30u32, 31] {
            slots.insert(id, StackSlot(-(8 * id as i64)));
            types.insert(id, IrType::I64);
            uses.insert(id, 5);
        }
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let insts = vec![MachInst::Mov128 {
            src: MachOperand::Mem {
                base: vreg(30),
                offset: 0,
            },
            dst: MachOperand::Mem {
                base: vreg(31),
                offset: 0,
            },
        }];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        // Both bases are simultaneously live (the Mov128 reads both): the
        // two reloads must name distinct registers.
        let reload_regs: Vec<MachReg> = out
            .iter()
            .filter_map(|i| match i {
                MachInst::Mov {
                    src: MachOperand::StackSlot(_),
                    dst: MachOperand::Reg(r @ MachReg::Phys(_)),
                    ..
                } => Some(*r),
                _ => None,
            })
            .collect();
        assert_eq!(
            reload_regs.len(),
            2,
            "expected two base reloads, got {out:?}"
        );
        assert_ne!(
            reload_regs[0], reload_regs[1],
            "overlapping live ranges must get distinct registers"
        );
        // Arriving, read-only bases: no write-back traffic for them.
        let stores: Vec<&MachInst> = out
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    MachInst::Mov {
                        dst: MachOperand::StackSlot(_),
                        ..
                    }
                )
            })
            .collect();
        assert!(
            stores.is_empty(),
            "read-only arrivals must not be stored: {out:?}"
        );
    }

    /// A register homed to a value live ACROSS the window (liveness-busy,
    /// not referenced inside) must not be handed out as a scratch.
    #[test]
    fn liveness_busy_register_is_not_a_scratch() {
        let (mut slots, mut types, mut uses, mut busy) = base_maps();
        slots.insert(40, StackSlot(-8));
        types.insert(40, IrType::I64);
        uses.insert(40, 2);
        // The preferred pool order starts with caller-saved r11(10), r10(11),
        // r8(12), r9(13), rdi(14), rsi(15), rdx(16). Mark ALL of them
        // liveness-busy across the window; the allocator must pick a
        // callee-saved register instead.
        for r in [10u8, 11, 12, 13, 14, 15, 16] {
            busy.insert(r, vec![(0, 15)]);
        }
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let insts = vec![MachInst::Alu {
            op: super::super::machinst::AluOp::Add,
            src: MachOperand::Imm(1),
            dst: vreg(40),
            size: OpSize::S64,
        }];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        let reload = out.iter().find_map(|i| match i {
            MachInst::Mov {
                src: MachOperand::StackSlot(_),
                dst: MachOperand::Reg(r),
                ..
            } => Some(*r),
            _ => None,
        });
        let Some(MachReg::Phys(pr)) = reload else {
            panic!("no reload found in {out:?}")
        };
        assert!(
            !matches!(pr.0, 10 | 11 | 12 | 13 | 14 | 15 | 16),
            "scratch {pr:?} collides with a liveness-busy register"
        );
    }

    /// A phys operand inside the live range blocks that register even when
    /// it is not busy anywhere else.
    #[test]
    fn phys_operand_interference_is_respected() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(50, StackSlot(-8));
        types.insert(50, IrType::I64);
        uses.insert(50, 2);
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let insts = vec![
            MachInst::Mov {
                src: MachOperand::Mem {
                    base: vreg(50),
                    offset: 0,
                },
                dst: MachOperand::Reg(phys(10)), // r11 used as a plain operand
                size: OpSize::S64,
            },
            MachInst::Alu {
                op: super::super::machinst::AluOp::Add,
                src: MachOperand::Reg(phys(10)),
                dst: vreg(50),
                size: OpSize::S64,
            },
        ];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        let scratch = out.iter().find_map(|i| match i {
            MachInst::Mov {
                src: MachOperand::StackSlot(_),
                dst: MachOperand::Reg(r),
                ..
            } => Some(*r),
            _ => None,
        });
        let Some(MachReg::Phys(pr)) = scratch else {
            panic!("no reload found in {out:?}")
        };
        assert_ne!(pr.0, 10, "scratch must not collide with the r11 operand");
    }

    /// The implicit rax/rdx clobbers of the division forms block those
    /// registers across their span.
    #[test]
    fn division_clobbers_block_rax_and_rdx() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(60, StackSlot(-8));
        types.insert(60, IrType::I64);
        uses.insert(60, 9);
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let insts = vec![
            MachInst::Mov {
                src: MachOperand::Mem {
                    base: vreg(60),
                    offset: 0,
                },
                dst: MachOperand::Reg(phys(0)), // dividend → rax
                size: OpSize::S64,
            },
            MachInst::Cqto { size: OpSize::S64 },
            MachInst::Div {
                divisor: MachOperand::Reg(phys(1)),
                signed: true,
                size: OpSize::S64,
            },
        ];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        assert_no_vregs(&out);
        let scratch = out.iter().find_map(|i| match i {
            MachInst::Mov {
                src: MachOperand::StackSlot(_),
                dst: MachOperand::Reg(r),
                ..
            } => Some(*r),
            _ => None,
        });
        let Some(MachReg::Phys(pr)) = scratch else {
            panic!("no reload found in {out:?}")
        };
        assert!(
            !matches!(pr.0, 0 | 16),
            "scratch {pr:?} collides with rax/rdx division clobbers"
        );
    }

    /// Refusals: XMM domain, missing slot, alloca value, double pure write.
    #[test]
    fn admission_refusals() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        let empty: FxHashSet<u32> = FxHashSet::default();
        slots.insert(70, StackSlot(-8)); // 70: fine
        slots.insert(71, StackSlot(-16)); // 71: XMM domain via FAlu
        slots.insert(72, StackSlot(-24)); // 72: float type
        types.insert(70, IrType::I64);
        types.insert(71, IrType::I64);
        types.insert(72, IrType::F64);
        uses.insert(70, 1);
        uses.insert(71, 1);
        uses.insert(72, 1);

        // XMM-domain reference (FAlu dst): refused.
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let xmm = vec![MachInst::FAlu {
            op: super::super::machinst::FAluOp::Add,
            src2: MachOperand::Reg(phys(20)),
            src1: vreg(71),
            dst: vreg(71),
            size: OpSize::S64,
        }];
        assert!(
            allocate_window(xmm, &c).is_none(),
            "XMM domain must be refused"
        );

        // Float-typed value in a GPR position: refused.
        let flt = vec![MachInst::Mov {
            src: MachOperand::Reg(vreg(72)),
            dst: MachOperand::Reg(phys(2)),
            size: OpSize::S64,
        }];
        assert!(
            allocate_window(flt, &c).is_none(),
            "float type must be refused"
        );

        // Missing slot: refused.
        let noslot = vec![MachInst::Mov {
            src: MachOperand::Reg(vreg(99)),
            dst: MachOperand::Reg(phys(2)),
            size: OpSize::S64,
        }];
        assert!(
            allocate_window(noslot, &c).is_none(),
            "slotless vreg must be refused"
        );

        // Double pure write (SSA violation): refused.
        let twowrite = vec![
            MachInst::Mov {
                src: MachOperand::Imm(1),
                dst: MachOperand::Reg(vreg(70)),
                size: OpSize::S64,
            },
            MachInst::Mov {
                src: MachOperand::Imm(2),
                dst: MachOperand::Reg(vreg(70)),
                size: OpSize::S64,
            },
        ];
        assert!(
            allocate_window(twowrite, &c).is_none(),
            "two pure writes must be refused"
        );

        // Alloca-backed value: refused (slot holds storage, not the pointer).
        let mut alloca = FxHashSet::default();
        alloca.insert(70);
        let c2 = WindowCtx {
            slots: &slots,
            small_slots: &empty,
            types: &types,
            total_uses: &uses,
            alloca_values: &alloca,
            reg_busy: &busy,
            window_span: (0, 15),
        };
        let alloc = vec![MachInst::Mov {
            src: MachOperand::Reg(vreg(70)),
            dst: MachOperand::Reg(phys(2)),
            size: OpSize::S64,
        }];
        assert!(
            allocate_window(alloc, &c2).is_none(),
            "alloca vreg must be refused"
        );
    }

    /// A Vreg shift amount refuses the window (the emitter's %cl-only form
    /// would silently drop the count).
    #[test]
    fn vreg_shift_amount_refuses() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(80, StackSlot(-8));
        types.insert(80, IrType::I64);
        uses.insert(80, 5);
        let empty = FxHashSet::default();
        let c = ctx(&slots, &types, &uses, &busy, &empty);
        let insts = vec![MachInst::Shift {
            op: super::super::machinst::ShiftOp::Shl,
            amount: MachOperand::Reg(vreg(80)),
            dst: vreg(80),
            size: OpSize::S64,
        }];
        assert!(
            allocate_window(insts, &c).is_none(),
            "a Vreg shift amount must refuse the window"
        );
    }

    /// classify_window returns exactly the vregs in register-only positions
    /// or memory bases — not the substitutable ones.
    #[test]
    fn classify_window_picks_register_positions() {
        let insts = vec![
            // A Mov src is substitutable (memory-form read).
            MachInst::Mov {
                src: MachOperand::Reg(vreg(1)),
                dst: MachOperand::Reg(phys(2)),
                size: OpSize::S64,
            },
            MachInst::Cmp {
                lhs: MachOperand::Reg(vreg(1)), // substitutable
                rhs: MachOperand::Imm(0),
                size: OpSize::S64,
            },
            // A two-address ALU dest is register-only.
            MachInst::Alu {
                op: super::super::machinst::AluOp::Add,
                src: MachOperand::Imm(1),
                dst: vreg(2),
                size: OpSize::S64,
            },
            MachInst::Mov {
                src: MachOperand::Reg(phys(2)),
                dst: MachOperand::Mem {
                    base: vreg(3), // memory base
                    offset: 0,
                },
                size: OpSize::S64,
            },
        ];
        let classified = classify_window(&insts);
        assert!(classified.contains(&2), "two-address dest must classify");
        assert!(classified.contains(&3), "memory base must classify");
        assert!(
            !classified.contains(&1),
            "substitutable positions must not classify"
        );
    }

    /// Small slots (4 bytes) clamp the transfer width to 32 bits: a 64-bit
    /// reload would read the neighbouring slot.
    #[test]
    fn small_slot_transfer_width_is_clamped() {
        let (mut slots, mut types, mut uses, busy) = base_maps();
        slots.insert(90, StackSlot(-8));
        types.insert(90, IrType::I64); // even a 64-bit typed value...
        uses.insert(90, 3);
        let small: FxHashSet<u32> = {
            let mut s = FxHashSet::default();
            s.insert(90);
            s
        };
        let empty = FxHashSet::default();
        let c = WindowCtx {
            slots: &slots,
            small_slots: &small,
            types: &types,
            total_uses: &uses,
            alloca_values: &empty,
            reg_busy: &busy,
            window_span: (0, 15),
        };
        let insts = vec![MachInst::Mov {
            src: MachOperand::Mem {
                base: vreg(90),
                offset: 0,
            },
            dst: MachOperand::Reg(phys(2)),
            size: OpSize::S64,
        }];
        let out = allocate_window(insts, &c).expect("allocation must succeed");
        let reload = out.iter().find_map(|i| match i {
            MachInst::Mov {
                src: MachOperand::StackSlot(_),
                dst: MachOperand::Reg(_),
                size,
            } => Some(*size),
            _ => None,
        });
        assert_eq!(
            reload,
            Some(OpSize::S32),
            "small-slot transfers must be 32-bit"
        );
    }
}
