//! Linear-scan register allocator.
//!
//! GPR: (1) callee-saved for values live *across* a call (`start < cp < end`)
//! except cheap remats; (2) caller-saved for the rest (i686: per-reg scratch
//! filter); (3) leftover callee-saved for call-free overflow.
//!
//! Copy groups with pairwise-disjoint intervals share a home. Loop-carried
//! phi dests may steal a cold callee-saved on the wide AArch64 pool only.
//! `detect_phi_coalesce_groups` is shared with stack-slot coalescing.

use super::live_range::{self, LinearScanAllocator};
use super::liveness::{
    compute_live_intervals, for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction, LiveInterval, LivenessResult,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysReg(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccumulatorOperandOrder {
    LhsFirst,
    AccumulatorCentric,
}

#[derive(Debug, Clone, Copy)]
pub struct AccumulatorPolicy {
    pub operand_order: AccumulatorOperandOrder,
    pub return_consumes_accumulator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccumulatorAssignment {
    pub value_id: u32,
    pub def_point: u32,
    pub consume_point: u32,
}

/// Build verified accumulator assignments under the target's evaluation-order
/// contract. Stack layout no longer owns this decision; it consumes the
/// allocator result. Program points use the same instruction+terminator order
/// as liveness.
pub fn analyze_accumulator_assignments(
    func: &IrFunction,
    policy: AccumulatorPolicy,
) -> Vec<AccumulatorAssignment> {
    let lhs_first = matches!(policy.operand_order, AccumulatorOperandOrder::LhsFirst);
    let candidates = crate::backend::stack_layout::copy_coalescing::compute_immediately_consumed(
        func, lhs_first,
    );
    if candidates.is_empty() {
        return Vec::new();
    }

    // Fused div/rem pair tails emit no code — their dest is NOT staged into
    // the accumulator at its IR def point (it was stored by the head). An
    // accumulator chain starting from a tail dest would let the consumer
    // read a stale %eax.
    let divrem = match divrem_target_for_current_arch() {
        Some(t) => compute_i686_divrem_pairs(func, t),
        None => return Vec::new(),
    };
    if !divrem.tail_dests.is_empty() {
        let filtered: Vec<u32> = candidates
            .into_iter()
            .filter(|c| !divrem.tail_dests.contains(c))
            .collect();
        if filtered.is_empty() {
            return Vec::new();
        }
        return analyze_accumulator_assignments_impl(func, policy, &filtered);
    }
    analyze_accumulator_assignments_impl(
        func,
        policy,
        &candidates.into_iter().collect::<Vec<u32>>(),
    )
}

fn analyze_accumulator_assignments_impl(
    func: &IrFunction,
    policy: AccumulatorPolicy,
    candidates: &[u32],
) -> Vec<AccumulatorAssignment> {
    let candidates: Vec<u32> = candidates.to_vec();

    let mut defs: FxHashMap<u32, u32> = FxHashMap::default();
    let mut uses: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut pp = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                defs.insert(dest.0, pp);
            }
            inst.for_each_used_value(|id| uses.entry(id).or_default().push(pp));
            pp += 1;
        }
        block
            .terminator
            .for_each_used_value(|id| uses.entry(id).or_default().push(pp));
        pp += 1;
    }
    let mut out = Vec::new();
    for value_id in candidates {
        let (Some(&def_point), Some(points)) = (defs.get(&value_id), uses.get(&value_id)) else {
            continue;
        };
        if points.len() != 1 || points[0] != def_point + 1 {
            continue;
        }
        // 64-bit targets do not consume scalar returns from the accumulator.
        if !policy.return_consumes_accumulator {
            let is_return = func.blocks.iter().any(|b| {
                matches!(&b.terminator, Terminator::Return(Some(Operand::Value(v))) if v.0 == value_id)
            });
            if is_return {
                continue;
            }
        }
        out.push(AccumulatorAssignment {
            value_id,
            def_point,
            consume_point: points[0],
        });
    }
    out.sort_unstable_by_key(|a| (a.def_point, a.value_id));
    verify_accumulator_assignments(func, &out);
    out
}

fn verify_accumulator_assignments(func: &IrFunction, assignments: &[AccumulatorAssignment]) {
    if assignments.is_empty() {
        return;
    }
    let mut defs = FxHashMap::default();
    let mut uses: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut pp = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                defs.insert(dest.0, pp);
            }
            inst.for_each_used_value(|id| uses.entry(id).or_default().push(pp));
            pp += 1;
        }
        block
            .terminator
            .for_each_used_value(|id| uses.entry(id).or_default().push(pp));
        pp += 1;
    }
    let mut seen = FxHashSet::default();
    for a in assignments {
        assert!(
            seen.insert(a.value_id),
            "duplicate accumulator assignment for v{}",
            a.value_id
        );
        assert_eq!(
            defs.get(&a.value_id).copied(),
            Some(a.def_point),
            "bad accumulator def point for v{}",
            a.value_id
        );
        assert_eq!(
            uses.get(&a.value_id).map(Vec::as_slice),
            Some([a.consume_point].as_slice()),
            "accumulator value v{} is not single-use",
            a.value_id
        );
        assert_eq!(
            a.consume_point,
            a.def_point + 1,
            "accumulator value v{} is not adjacent",
            a.value_id
        );
    }
}

pub struct RegAllocResult {
    pub assignments: FxHashMap<u32, PhysReg>,
    pub accumulator_assignments: Vec<AccumulatorAssignment>,
    pub used_regs: Vec<PhysReg>,
    pub caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>>,
    pub liveness: Option<LivenessResult>,
}

/// Whether x86 can preserve incoming parameters directly in caller-saved homes.
/// ParamRefs must execute in the entry prefix before any generated instruction
/// can clobber ABI argument registers, and the function must be call-free.
/// RISC-V analog of [`x86_param_caller_homes_safe`]: integer ABI slot i is
/// register a_i, so a call-free function whose ParamRefs all run in the
/// entry prefix (before any generated instruction can clobber a0–a7) can
/// keep its scalar parameters directly in their incoming registers — the
/// ParamRef emits nothing at all when the home matches the incoming reg.
pub fn riscv_param_caller_homes_safe(func: &IrFunction) -> bool {
    if func.blocks.is_empty() || env_on("CCC_NO_LEAF_PARAM_GPR") {
        return false;
    }
    if func.blocks.len() > 1 && func.params.len() > 8 {
        return false;
    }
    if func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::InlineAsm { .. }
            )
        })
    }) {
        return false;
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi != 0 {
            if block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::ParamRef { .. }))
            {
                return false;
            }
            continue;
        }
        let mut seen_code = false;
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { .. } | Instruction::ParamRef { .. } if !seen_code => {}
                Instruction::ParamRef { .. } => return false,
                _ => seen_code = true,
            }
        }
    }
    true
}

pub fn x86_param_caller_homes_safe(func: &IrFunction) -> bool {
    if func.blocks.is_empty() || env_on("CCC_NO_LEAF_PARAM_GPR") {
        return false;
    }
    // Multi-block expansion is limited to the six SysV register arguments.
    // Stack arguments need a stable entry-RSP frame model; moving those homes
    // changes their offsets (the nine-argument regression catches this).
    if func.blocks.len() > 1 && func.params.len() > 6 {
        return false;
    }
    if func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::InlineAsm { .. }
            )
        })
    }) {
        return false;
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi != 0 {
            if block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::ParamRef { .. }))
            {
                return false;
            }
            continue;
        }
        let mut seen_code = false;
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { .. } | Instruction::ParamRef { .. } if !seen_code => {}
                Instruction::ParamRef { .. } => return false,
                _ => seen_code = true,
            }
        }
    }
    true
}

pub struct RegAllocConfig {
    pub available_regs: Vec<PhysReg>,
    pub accumulator_policy: AccumulatorPolicy,
    pub caller_saved_regs: Vec<PhysReg>,
    /// Subset of `caller_saved_regs` that a call's argument staging writes
    /// (SysV AMD64: rdi/rsi/rdx/r8/r9; AArch64: x4..x7 and the x8 indirect
    /// result). A value used as a call argument must never be homed here:
    /// the staging materialises earlier arguments into those exact registers
    /// before reading this value, so its home would already be clobbered
    /// (`printf("%d %d", add(3,4), mul(3,4))` read the mul result out of the
    /// format-string register). Empty on backends with no arg-register caller-
    /// saved pool (i686, RISC-V).
    pub call_arg_regs: Vec<PhysReg>,
    /// Caller-saved registers the INDIRECT-call staging additionally writes
    /// before reading arguments (SysV AMD64: r10 holds the callee address from
    /// `emit_call_spill_fptr` until the `call *%r10`). A value used as an
    /// argument to a `CallIndirect` must avoid these too — otherwise the
    /// function-pointer spill clobbers its home before it is staged
    /// (`ops[op](a+i)` passed the callee address as its own argument).
    pub indirect_target_regs: Vec<PhysReg>,
    pub allow_inline_asm_regalloc: bool,
    pub xmm_regs: Vec<PhysReg>,
    pub never_materialized: FxHashSet<u32>,
    /// Backend-folded GEP index consumers: map of `index value id` → the
    /// GEP-dest value ids whose Load/Store consumes the index through an
    /// indexed addressing form. The IR records no use of the index at that
    /// point (the offset computation was folded away), so the allocator must
    /// extend the index's live interval to the consumer's own interval end —
    /// otherwise the index's register is free for reuse and the emitted
    /// `[base, index, lsl #N]` reads whatever moved in (reproduced: fa[i]
    /// stores landing on fa[seed] on aarch64 -O0/-O2).
    ///
    /// ONLY backends that actually EMIT the indexed form (overriding
    /// emit_load_indexed/emit_store_indexed — currently arm alone) may pass a
    /// non-empty map. x86-64/i686 return false from the default hooks and
    /// re-materialise the skipped GEP at the load (IR-visible uses intact);
    /// extending there only adds register pressure (it regressed
    /// check_gpr_leaf_param_codegen::pointer_mix when applied globally —
    /// session-23 audit of the Agent-B patch).
    pub folded_index_uses: FxHashMap<u32, Vec<u32>>,
    /// ABI-preferred homes (e.g. an incoming ParamRef already in `%rdi`).
    /// Hints never override `follow_value` and are honored only when the
    /// physical register belongs to the current allocation wave.
    pub reg_hints: FxHashMap<u32, PhysReg>,
    /// Leaf-function home policy (x86-64 only today).
    ///
    /// Session 28's "hot loop-carried values join Phase 1" rule hands every
    /// heavily-used loop value a *callee-saved* home even when the function
    /// never calls anything. On a call-free leaf that buys nothing — a
    /// caller-saved home is exactly as safe — and costs a `push`/`pop` pair
    /// per register plus a bigger frame (kernel corpus: `maxv` carried
    /// rbx/r12/r13 = 6 prologue/epilogue instructions for a 4-instruction
    /// loop body; `cntz`, `bswp32`, `scmp`, `ffs1` likewise). GCC, Clang and
    /// ICX all allocate such leaves from the volatile pool first.
    ///
    /// When set AND the function has no call points, hot loop values skip
    /// the Phase-1 promotion: they are scanned in Phase 2 (caller-saved,
    /// priority-evicting — loop depth weights keep them ahead of span-1
    /// temps) and only the *overflow* takes callee-saved homes in Phase 2c,
    /// i.e. the prologue saves exactly the registers the loop actually
    /// needs. Kill switch: `CCC_NO_LEAF_CALLER_HOME`.
    pub leaf_caller_saved_homes: bool,
}

fn env_on(name: &'static str) -> bool {
    std::env::var_os(name).is_some()
}

fn interval_map(liveness: &LivenessResult) -> FxHashMap<u32, (u32, u32)> {
    let mut m = FxHashMap::default();
    m.reserve(liveness.intervals.len());
    for iv in &liveness.intervals {
        m.insert(iv.value_id, (iv.start, iv.end));
    }
    m
}

fn intervals_overlap(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// Linear merge-style interference test for sorted hole-aware segment sets.
/// Uses the allocator's half-open boundary convention: one value dying at the
/// exact point another is born may hand the register directly to it.
fn segment_sets_overlap(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    let (mut ai, mut bi) = (0usize, 0usize);
    while ai < a.len() && bi < b.len() {
        if intervals_overlap(a[ai], b[bi]) {
            return true;
        }
        if a[ai].1 <= b[bi].0 {
            ai += 1;
        } else {
            bi += 1;
        }
    }
    false
}

/// Merge two sorted segment sets into a normalized union. Adjacent pieces are
/// combined: no candidate can use the zero-width boundary between them.
fn insert_segment_union(into: &mut Vec<(u32, u32)>, added: &[(u32, u32)]) {
    let mut all: Vec<(u32, u32)> = Vec::with_capacity(into.len() + added.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < into.len() || j < added.len() {
        let next = if j == added.len() || (i < into.len() && into[i] <= added[j]) {
            let value = into[i];
            i += 1;
            value
        } else {
            let value = added[j];
            j += 1;
            value
        };
        if let Some(last) = all.last_mut() {
            if next.0 <= last.1 {
                last.1 = last.1.max(next.1);
                continue;
            }
        }
        all.push(next);
    }
    *into = all;
}

/// RA-05: attach hole-aware live coverage to scan ranges so the linear scan's
/// interference test can see through liveness holes (values on mutually
/// exclusive diamond/switch arms — the xmltok/inflate 12×/15× stack-memory
/// pathology). Coverage is per coalesce-group OWNER (union of the leader's
/// and every member's segments) exactly like the Phase 2f residual filler:
/// `propagate_coalesce_members` later hands the leader's register to every
/// member, so the scan must reason about the whole web's coverage.
///
/// Fail-closed: a value keeps fat-envelope semantics when it has no segment
/// data or its segments do not lie inside the scan's fat envelope (merged
/// envelopes are the only case where the envelope exceeds the raw interval).
fn attach_scan_segments(
    ranges: &mut [crate::backend::live_range::LiveRange],
    liveness: &LivenessResult,
    coalesce_member_of: &FxHashMap<u32, u32>,
) {
    if ranges.is_empty() {
        return;
    }
    let owner_of = |v: u32| coalesce_member_of.get(&v).copied().unwrap_or(v);
    let mut owned: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for seg in &liveness.segments {
        let owner = owner_of(seg.value_id);
        owned.entry(owner).or_default().push((seg.start, seg.end));
    }
    if owned.is_empty() {
        return;
    }
    for pieces in owned.values_mut() {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_segment_union(pieces, &source);
    }
    for range in ranges.iter_mut() {
        let Some(segs) = owned.get(&range.value_id) else {
            continue;
        };
        if segs.is_empty() {
            continue;
        }
        // The union must lie inside the range's fat envelope; otherwise the
        // envelope came from a different construction (defensive) and the
        // range keeps conservative fat semantics.
        let inside = segs.first().is_some_and(|&(s, _)| s >= range.start)
            && segs.last().is_some_and(|&(_, e)| e <= range.end);
        if inside {
            range.set_segments(segs.clone());
        }
    }
}

/// Live *across* a clobber: defined before it, used after it.
/// Born at a call (retval) or dying at a call (arg) may use caller-saved.
#[inline]
fn spans_any_call(iv: &LiveInterval, call_points: &[u32]) -> bool {
    let idx = call_points.partition_point(|&cp| cp <= iv.start);
    idx < call_points.len() && call_points[idx] < iv.end
}

/// Inclusive — i686 scratch may clobber while the insn still reads the value.
#[inline]
fn overlaps_inclusive(iv: &LiveInterval, points: &[u32]) -> bool {
    let idx = points.partition_point(|&p| p < iv.start);
    idx < points.len() && points[idx] <= iv.end
}

/// Like `overlaps_inclusive`, but hazards at the interval's OWN definition
/// point are birth, not clobber: every emitter shape stages its early writes
/// (scratch relays, `%ecx` pointer materialisation) BEFORE the final
/// destination write that constitutes the value's birth, so a hazard exactly
/// at `iv.start` can never destroy a value that does not exist yet. This is
/// what lets a `divl`-born remainder claim `%edx` and a GEP-born pointer
/// claim `%ecx` across their own defining instruction.
#[inline]
fn overlaps_inclusive_skip_birth(iv: &LiveInterval, points: &[u32]) -> bool {
    let idx = points.partition_point(|&p| p < iv.start);
    // Skip a hazard exactly at the def point (birth), then require any
    // remaining hazard to fall within (start, end].
    let mut idx = idx;
    if idx < points.len() && points[idx] == iv.start {
        idx += 1;
    }
    idx < points.len() && points[idx] <= iv.end
}

// ── i686 div/rem same-block pair analysis ────────────────────────────────────

/// Deterministic div/rem pair analysis shared by the i686 emitter and the
/// register allocator's hazard scan. Both derive it from the same IR, so the
/// two views can never disagree.
///
/// A pair is a `URem`/`UDiv` (or `SRem`/`SDiv`) couple in the SAME block with
/// IDENTICAL operands. One `divl`/`idivl` computes both results: the head
/// (first in emission order) emits the division and stores BOTH results
/// immediately — its own from its natural output register (`%edx` for rem,
/// `%eax` for div) and the partner's from the other output — while the tail
/// emits NOTHING (its result was already stored at the head). Soundness does
/// not depend on what lies between the two instructions: SSA operands cannot
/// change, and both stores happen at the head, before anything else runs.
///
/// Constant-RHS divisions never pair: at -O2 the emitters replace constant
/// divisions with magic-number sequences that never execute the hardware
/// divide, so a fused pair there would be dead code feeding nothing (and
/// keeping the emitter side unconditional is the only model the register
/// allocator can share soundly — the RA always models exactly what every
/// optimisation level emits for these pairs).
///
/// The `target` gate selects the operand width class per backend: i686 pairs
/// 32-bit GPR ops, x86-64 additionally pairs 64-bit ops (divq/idivq), and
/// AArch64 pairs both (sdiv/udiv + msub shape). RISC-V never pairs: div and
/// rem are separate single-uop instructions there and the sdiv+msub shape
/// would be three operations instead of two.
///
/// The function name keeps the historical `I686` prefix in the pair struct
/// (the shape originated there); the shared RA consumes pairs for every
/// target the gate enables.
///
///
/// The greedy pairing is order-stable: scanning forward, each still-unpaired
/// division pairs with the next unpaired compatible division in the same
/// block. Compatibility requires identical `lhs`/`rhs` operands and matching
/// signedness (`URem` with `UDiv`/`URem`, `SRem` with `SDiv`/`SRem`); mixing
/// flavours would change the executed instruction.
/// Per-backend div/rem fusion gate (see `compute_i686_divrem_pairs`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DivRemTarget {
    I686,
    X86_64,
    AArch64,
}

/// The fusion target for the CURRENT compilation unit, or None where fusion
/// never pays (RISC-V and any non-registered target).
pub(crate) fn divrem_target_for_current_arch() -> Option<DivRemTarget> {
    match crate::common::types::target_elf_machine() {
        crate::backend::elf::EM_386 => Some(DivRemTarget::I686),
        crate::backend::elf::EM_X86_64 => Some(DivRemTarget::X86_64),
        crate::backend::elf::EM_AARCH64 => Some(DivRemTarget::AArch64),
        _ => None,
    }
}

pub(crate) struct I686DivRemPairs {
    /// Dest value-ids of tail instructions — the emitter skips them entirely.
    pub tail_dests: FxHashSet<u32>,
    /// Head dest value-id -> (partner dest, partner result is the quotient in
    /// `%eax`). `false` means the partner wants the remainder in `%edx`.
    pub head_partners: FxHashMap<u32, (u32, bool)>,
    /// `(block_idx, inst_idx)` of tail instructions, for the hazard scan
    /// (which numbers program points with the same walk).
    pub tail_points: FxHashSet<(usize, usize)>,
    /// Tail dest value-id -> the head's PROGRAM POINT. The tail's result is
    /// physically born at the head (dual-store); liveness intervals for tail
    /// dests must start there, or register homes / stack slots could be
    /// shared with values living between head and tail.
    pub head_point_of_tail: FxHashMap<u32, u32>,
}

pub(crate) fn compute_i686_divrem_pairs(
    func: &IrFunction,
    target: DivRemTarget,
) -> I686DivRemPairs {
    let mut pairs = I686DivRemPairs {
        tail_dests: FxHashSet::default(),
        head_partners: FxHashMap::default(),
        tail_points: FxHashSet::default(),
        head_point_of_tail: FxHashMap::default(),
    };
    if std::env::var_os("CCC_NO_IR_DIVREM").is_some() {
        return pairs;
    }

    // Program-point numbering identical to liveness/the hazard scan:
    // one point per instruction, one per terminator.
    let mut block_start: Vec<u32> = Vec::with_capacity(func.blocks.len());
    let mut pt = 0u32;
    for block in &func.blocks {
        block_start.push(pt);
        pt += block.instructions.len() as u32 + 1;
    }

    let pairs_64 = target != DivRemTarget::I686;
    let is_gpr32_ty = |ty: &IrType| {
        matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::Ptr
        ) || (pairs_64 && matches!(ty, IrType::I64 | IrType::U64))
    };
    let div_flavor = |op: IrBinOp| -> Option<bool> {
        // Some(true) = quotient (div), Some(false) = remainder, None = not div-like
        match op {
            IrBinOp::UDiv | IrBinOp::SDiv => Some(true),
            IrBinOp::URem | IrBinOp::SRem => Some(false),
            _ => None,
        }
    };
    let is_signed = |op: IrBinOp| matches!(op, IrBinOp::SDiv | IrBinOp::SRem);

    for (bi, block) in func.blocks.iter().enumerate() {
        // Collect this block's div-like instructions.
        let mut cands: Vec<(usize, IrBinOp, &Operand, &Operand, bool, u32)> = Vec::new();
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::BinOp {
                dest,
                op,
                ty,
                lhs,
                rhs,
                ..
            } = inst
            {
                if div_flavor(*op).is_some() && is_gpr32_ty(ty) {
                    // i686: constant divisors pair too: the head folds the
                    // pair into ONE magic-number sequence (q in %eax, r in
                    // %edx, `emit_divrem_const_in_eax_edx`) or, at -Os, one
                    // `divl $imm`-staged division. Both clobber exactly the
                    // {%eax,%ecx,%edx} set the RA model already charges a
                    // constant-divisor division with, and the tail still
                    // emits nothing, so the model stays exact. Other
                    // targets keep the non-constant rule (their IR-level
                    // div_by_const pass owns constant divisors).
                    if target == DivRemTarget::I686 || !matches!(rhs, Operand::Const(_)) {
                        cands.push((ii, *op, lhs, rhs, is_signed(*op), dest.0));
                    }
                }
            }
        }
        if cands.len() < 2 {
            continue;
        }
        let mut used = vec![false; cands.len()];
        for i in 0..cands.len() {
            if used[i] {
                continue;
            }
            let (_, _, lhs_i, rhs_i, signed_i, dest_i) = &cands[i];
            // Nearest unused compatible partner ahead in this block.
            let mut mate: Option<usize> = None;
            for j in (i + 1)..cands.len() {
                if used[j] {
                    continue;
                }
                let (_, _, lhs_j, rhs_j, signed_j, _) = &cands[j];
                if signed_i == signed_j && lhs_i == lhs_j && rhs_i == rhs_j {
                    mate = Some(j);
                    break;
                }
            }
            let Some(j) = mate else { continue };
            used[i] = true;
            used[j] = true;
            let (_, op_j, _, _, _, dest_j) = &cands[j];
            // Head = i (earlier), tail = j. The partner's result register:
            // quotient ops read %eax, remainder ops read %edx.
            let partner_from_eax = div_flavor(*op_j) == Some(true);
            pairs
                .head_partners
                .insert(*dest_i, (*dest_j, partner_from_eax));
            pairs.tail_dests.insert(*dest_j);
            pairs.tail_points.insert((bi, cands[j].0));
            pairs
                .head_point_of_tail
                .insert(*dest_j, block_start[bi] + cands[i].0 as u32);
        }
    }
    pairs
}

/// DivRem pair tails are physically born at their HEAD's program point (the
/// head's dual-store writes the tail's dest register/slot). Every consumer
/// of liveness — register homes, stack-slot sharing, coalescing — must see
/// the tail dest live from the head onward, otherwise the home/slot can be
/// handed to another value in the head..tail window and silently clobbered.
/// This patches both the fat intervals and the hole-aware segments, then
/// restores the segments' `(start, value_id)` sort order.
fn patch_divrem_tail_intervals(func: &IrFunction, liveness: &mut LivenessResult) {
    let Some(target) = divrem_target_for_current_arch() else {
        return;
    };
    let pairs = compute_i686_divrem_pairs(func, target);
    if pairs.head_point_of_tail.is_empty() {
        return;
    }
    let head_of = &pairs.head_point_of_tail;
    for iv in &mut liveness.intervals {
        if let Some(&hp) = head_of.get(&iv.value_id) {
            if hp < iv.start {
                iv.start = hp;
            }
        }
    }
    for seg in &mut liveness.segments {
        if let Some(&hp) = head_of.get(&seg.value_id) {
            if hp < seg.start {
                seg.start = hp;
            }
        }
    }
    liveness
        .segments
        .sort_by(|a, b| (a.start, a.value_id).cmp(&(b.start, b.value_id)));
}

/// One detected `t2 = t + v` tail of a mul-accumulate chain, where `t` is a
/// same-block i64 Mul whose ONLY use is this Add. The emitter fuses the whole
/// `res*base + val` expression at the Mul (the head): it computes the product
/// in %eax:%edx with the zero-high-half short shape, adds the (provably
/// zero-extended) addend in place, and stores ONLY the tail dest. The tail
/// emits nothing. The product `t` is dead-by-construction (single use) and is
/// never stored, eliminating the slot round-trip.
#[derive(Debug, Clone)]
pub(crate) struct MulAccChain {
    /// Dest of the head (Mul) instruction `t`.
    pub head_dest: u32,
    /// Dest of the tail (Add) instruction `t2`.
    pub tail_dest: u32,
    /// Program point of the head (Mul) instruction.
    pub head_point: u32,
    /// The Mul's lhs (res).
    pub lhs: Operand,
    /// The Mul's rhs (base): i64 constant with zero high half, or a Value.
    pub rhs: Operand,
    /// If the rhs is a single-use zext cast whose 32-bit source is defined
    /// before the head, its dest value-id (a VIRTUAL feeder candidate: the
    /// cast may emit nothing and the head references the source directly).
    pub rhs_feeder: Option<u32>,
    /// The Add's rhs (addend val): i64 constant with zero high half, or a
    /// Value that is a zext-widening cast dest (or a feeder).
    pub addend: Operand,
    /// Feeder candidate for the addend (same rules as `rhs_feeder`).
    pub addend_feeder: Option<u32>,
    /// True when the addend's defining zext cast sits BETWEEN head and tail:
    /// the addend is then only readable through its feeder source (the cast
    /// dest's slot is not yet written at the head point).
    pub addend_cast_after_head: bool,
    /// Single-use I64<->U64 no-op cast dests crossed by the operand
    /// canonicalization (rhs and/or addend). Their ONLY reader is the chain
    /// (the tail or the head operand position); when the chain fuses, that
    /// reader emits nothing, so these casts are DEAD and the emitter
    /// suppresses their slot-pair copies.
    pub dead_nop_casts: Vec<u32>,
}

impl MulAccChain {
    /// Feeder candidates (zext cast dests) feeding this chain.
    pub fn feeders(&self) -> impl Iterator<Item = u32> + '_ {
        self.rhs_feeder.into_iter().chain(self.addend_feeder)
    }
}

pub(crate) struct I686MulAccChains {
    /// All detected chains (the emitter resolves fusibility per chain AFTER
    /// register allocation, when homes/slots are final).
    pub chains: Vec<MulAccChain>,
    /// head (Mul) dest -> index into `chains`.
    pub head_of: FxHashMap<u32, usize>,
    /// tail (Add) dest -> index into `chains`.
    pub tail_of: FxHashMap<u32, usize>,
}

/// Same-block i64 mul-accumulate chain analysis — the parser hot shape
/// `res = res*base + val` (kstrtoull/simple_strtoull). Deterministic,
/// derived from the same IR by emitter and RA (mirrors the divrem-pair
/// contract). Conditions:
///
/// - Head: an `I64/U64 Mul` whose result `t` has exactly ONE use in the
///   whole function: the tail Add. (Otherwise the product must exist
///   independently and cannot be folded into the fused store.)
/// - Tail: an `I64/U64 Add(t, v)` in the same block, after the head.
/// - The rhs (base) and the addend (val) must each be either an i64 constant
///   with a provably zero high half (< 2^32), or the dest of a
///   ZERO-extending cast (u8/u16/u32/ptr -> i64/u64) — the high half is then
///   provably zero, so the fused `adcl $0` and the 3-term product shape are
///   sound. Sign-extending casts and general 64-bit values reject the chain.
/// - A zext operand with a single function-wide use becomes a VIRTUAL
///   feeder candidate: its cast may emit nothing, and the head reads the
///   32-bit SOURCE directly. The source must be defined before the head
///   (SSA gives this for free for the rhs; for the addend the cast may sit
///   between head and tail, so the source's def point is checked).
///
/// Kill switch: `CCC_NO_MULACC` (both the RA patch and the emitter respect it).
pub(crate) fn compute_i686_mulacc_chains(func: &IrFunction) -> I686MulAccChains {
    let mut out = I686MulAccChains {
        chains: Vec::new(),
        head_of: FxHashMap::default(),
        tail_of: FxHashMap::default(),
    };
    if std::env::var_os("CCC_NO_MULACC").is_some() {
        return out;
    }

    // Program-point numbering identical to liveness/the hazard scan.
    let mut block_start: Vec<u32> = Vec::with_capacity(func.blocks.len());
    let mut pt = 0u32;
    for block in &func.blocks {
        block_start.push(pt);
        pt += block.instructions.len() as u32 + 1;
    }

    // Function-wide use counts (instructions + terminators).
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    let is_wide_ty = |ty: &IrType| matches!(ty, IrType::I64 | IrType::U64);
    let zext_from = |from_ty: &IrType, to_ty: &IrType| {
        matches!(
            (from_ty, to_ty),
            (
                IrType::U8 | IrType::U16 | IrType::U32 | IrType::Ptr,
                IrType::I64 | IrType::U64
            )
        )
    };
    // Constant with provably zero high half.
    let const_hi_zero = |op: &Operand| match op {
        Operand::Const(IrConst::Zero) => true,
        Operand::Const(c) => c.to_i64().is_some_and(|v| {
            let v = v as u128 & 0xFFFF_FFFF_FFFF_FFFF;
            v >> 32 == 0
        }),
        _ => false,
    };

    // Same-size wide reinterpretation casts (I64<->U64): bit-identical
    // values; their emitter materialisation is a slot-pair copy. The chain
    // analysis canonicalizes operands THROUGH them so a C sandwich like
    // `res*base + (u64)(s64)val` still exposes its real widening feeder.
    // SOUNDNESS: the lookthrough only crosses a no-op cast whose dest has a
    // SINGLE use — the chain consuming it. A multi-use no-op dest keeps
    // other readers, its copy must materialise, and that copy reads the
    // feeder's slot: crossing it would leave the copy reading the
    // never-written slot of a VIRTUALIZED feeder (use-after-death).
    // Crossed single-use no-op dests are recorded: they are dead when the
    // chain fuses (their one reader, the tail, emits nothing) and the
    // emitter then suppresses the copy. Depth-capped against pathological
    // chains; beyond the cap the operand resolves as-is.
    let mut wide_nop_casts: FxHashMap<u32, Operand> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } = inst
            {
                if is_wide_ty(from_ty) && is_wide_ty(to_ty) {
                    wide_nop_casts.insert(dest.0, src.clone());
                }
            }
        }
    }
    // (canonical operand, single-use no-op cast dests crossed on the way)
    let canon = |op: &Operand| -> (Operand, Vec<u32>) {
        let mut crossed = Vec::new();
        let mut cur = op.clone();
        for _ in 0..8 {
            match &cur {
                Operand::Value(v) => {
                    let single_use = use_count.get(&v.0).copied().unwrap_or(0) == 1;
                    match wide_nop_casts.get(&v.0) {
                        Some(next) if single_use => {
                            crossed.push(v.0);
                            cur = next.clone();
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        (cur, crossed)
    };

    for (bi, block) in func.blocks.iter().enumerate() {
        // Widening (to i64/u64) cast dests in this block:
        // dest -> (src operand, inst idx, is_zext). Sext casts qualify as
        // ADDEND feeders only (their high half is the sign replication).
        let mut widening_casts: FxHashMap<u32, (Operand, usize, bool)> = FxHashMap::default();
        // Def points of every value defined in this block (params are
        // entry-defined: point 0 dominates everything).
        let mut def_point: FxHashMap<u32, u32> = FxHashMap::default();
        // i64 Muls: (inst idx, dest, lhs, rhs).
        let mut muls: Vec<(usize, u32, Operand, Operand, Vec<u32>)> = Vec::new();
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    if is_wide_ty(to_ty)
                        && !is_wide_ty(from_ty)
                        && matches!(
                            from_ty,
                            IrType::U8
                                | IrType::U16
                                | IrType::U32
                                | IrType::Ptr
                                | IrType::I8
                                | IrType::I16
                                | IrType::I32
                        )
                    {
                        let z = zext_from(from_ty, to_ty);
                        widening_casts.insert(dest.0, (src.clone(), ii, z));
                    }
                    def_point.insert(dest.0, block_start[bi] + ii as u32);
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    ty,
                    lhs,
                    rhs,
                    ..
                } if is_wide_ty(ty) => {
                    def_point.insert(dest.0, block_start[bi] + ii as u32);
                    // Canonicalize the rhs through single-use I64<->U64
                    // no-op casts; the crossed dests die with the chain.
                    let (rhs_canon, rhs_dead_nops) = canon(rhs);
                    muls.push((ii, dest.0, lhs.clone(), rhs_canon, rhs_dead_nops));
                }
                Instruction::BinOp { dest, .. } => {
                    def_point.insert(dest.0, block_start[bi] + ii as u32);
                }
                _ => {
                    // Other defining instructions (Load, UnaryOp, Cmp, ...).
                    if let Some(dest) = inst.dest() {
                        def_point.insert(dest.0, block_start[bi] + ii as u32);
                    }
                }
            }
        }

        // Source of `op` defined (strictly) before `limit` in this block, or
        // defined elsewhere (params / other blocks) — always available.
        let src_available_before = |op: &Operand, limit: u32| -> bool {
            match op {
                Operand::Const(_) => true,
                Operand::Value(v) => match def_point.get(&v.0) {
                    Some(&dp) => dp < limit,
                    None => true, // defined in another block / param
                },
            }
        };

        // Resolve the MUL's RHS (base): zext-provable or zero-high constant
        // only — the fused 3-term product shape needs bhi = 0. `op` is
        // ALREADY canonicalized (I64<->U64 no-op casts stripped) by the
        // muls-table builder.
        let resolve_rhs = |op: &Operand, use_point: u32| -> Option<(Operand, Option<u32>)> {
            match op {
                Operand::Const(_) if const_hi_zero(op) => Some((op.clone(), None)),
                Operand::Value(v) => {
                    let single_use = use_count.get(&v.0).copied().unwrap_or(0) == 1;
                    if let Some((src, _cast_idx, is_zext)) = widening_casts.get(&v.0) {
                        if !is_zext {
                            return None; // sext base: bhi unprovable
                        }
                        // SSA guarantees the cast precedes the head (the
                        // Mul uses v), so the source is available there.
                        if single_use && src_available_before(src, use_point) {
                            Some((op.clone(), Some(v.0)))
                        } else {
                            // Materialized zext before the head: readable
                            // from the slot low half.
                            Some((op.clone(), None))
                        }
                    } else {
                        // Cross-block zext dest: the emitter's zext proof
                        // (zext_wide_values) gates it. Same-block non-cast
                        // defs are not provably zero-high.
                        match def_point.get(&v.0) {
                            Some(_) => None,
                            None => Some((op.clone(), None)),
                        }
                    }
                }
                _ => None,
            }
        };

        // Resolve the ADDEND (val): far more permissive — both halves are
        // readable from a materialized slot pair, so ANY wide value defined
        // (or materialized) before the head works; constants carry lo+hi;
        // single-use widening casts (zext OR sext) become feeder candidates.
        // `op` is ALREADY canonicalized by the tail-loop caller.
        let resolve_addend =
            |op: &Operand, use_point: u32, after_head: bool| -> Option<(Operand, Option<u32>)> {
                match op {
                    Operand::Const(c) if c.to_i64().is_some() => Some((op.clone(), None)),
                    Operand::Value(v) => {
                        let single_use = use_count.get(&v.0).copied().unwrap_or(0) == 1;
                        if let Some((src, cast_idx, _is_zext)) = widening_casts.get(&v.0) {
                            if single_use && src_available_before(src, use_point) {
                                Some((op.clone(), Some(v.0)))
                            } else if (block_start[bi] + *cast_idx as u32) < use_point {
                                // Multi-use widening cast before the head:
                                // materialized, both halves in its slot pair.
                                Some((op.clone(), None))
                            } else {
                                // Cast between head and tail, not a feeder: its
                                // slot is not yet written at the head point.
                                None
                            }
                        } else {
                            // Non-cast Value: sound iff defined before the head
                            // (materialized with a slot) or cross-block.
                            match def_point.get(&v.0) {
                                Some(&dp) if dp < use_point => Some((op.clone(), None)),
                                Some(_) => None,
                                None => Some((op.clone(), None)),
                            }
                        }
                    }
                    _ => None,
                }
            };

        // Tail Adds: t2 = Add(t, v).
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                ty,
                lhs,
                rhs,
                ..
            } = inst
            else {
                continue;
            };
            if !is_wide_ty(ty) {
                continue;
            }
            let Operand::Value(t) = lhs else { continue };
            // Head: a same-block i64 Mul defining t, before this Add.
            let Some(&(mul_idx, head_dest, ref head_lhs, ref head_rhs, ref rhs_dead_nops)) =
                muls.iter().find(|(mi, md, ..)| *md == t.0 && *mi < ii)
            else {
                continue;
            };
            // t's single use must be this Add.
            if use_count.get(&t.0).copied().unwrap_or(0) != 1 {
                continue;
            }
            let head_point = block_start[bi] + mul_idx as u32;
            // Addend: value use point is the head (the fused add reads it at
            // the head); after_head marks materializations sitting between
            // head and tail (unreadable at the head point). Computed on the
            // CANONICAL addend operand (through single-use I64<->U64 no-op
            // casts; the crossed dests die with the chain).
            let (addend_canon, addend_dead_nops) = canon(rhs);
            let after_head = match &addend_canon {
                Operand::Value(v) => widening_casts
                    .get(&v.0)
                    .is_some_and(|(_, ci, _)| *ci > mul_idx),
                _ => false,
            };
            let Some((addend, addend_feeder)) =
                resolve_addend(&addend_canon, head_point, after_head)
            else {
                continue;
            };
            // RHS: SSA guarantees the cast (if any) is before the head.
            let Some((rhs_op, rhs_feeder)) = resolve_rhs(head_rhs, head_point) else {
                continue;
            };
            let mut dead_nop_casts = rhs_dead_nops.clone();
            dead_nop_casts.extend(addend_dead_nops.iter().copied());
            let chain = MulAccChain {
                head_dest,
                tail_dest: dest.0,
                head_point,
                lhs: head_lhs.clone(),
                rhs: rhs_op,
                rhs_feeder,
                addend,
                addend_feeder,
                addend_cast_after_head: after_head,
                dead_nop_casts,
            };
            out.head_of.insert(head_dest, out.chains.len());
            out.tail_of.insert(dest.0, out.chains.len());
            out.chains.push(chain);
        }
    }
    out
}

/// Mul-acc chain tails are physically born at their HEAD's fused store; the
/// fused head also reads the feeder SOURCES (whose natural death is their
//  zext cast, potentially before the head). Patch both interval families
/// before any interval map is derived — the divrem-tail contract.
fn patch_mulacc_intervals(func: &IrFunction, liveness: &mut LivenessResult) {
    let chains = compute_i686_mulacc_chains(func);
    if chains.chains.is_empty() {
        return;
    }
    // Feeder cast dest -> source operand (re-walked here; the table keeps
    // only value-ids so the emitter can re-resolve against final homes).
    let mut feeder_cast_src: FxHashMap<u32, Operand> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } = inst
            {
                // ALL widening feeders (zext AND sext): the fused head reads
                // the 32-bit SOURCE directly in both cases — zext for the
                // rhs (bhi=0 proof), sext for the addend (AddendHi::Sext,
                // sarl replication). Restricting the source-liveness
                // extension to zext feeders left a use-after-death window
                // for SEXT addend feeders whose cast precedes the head:
                // the source's interval ended at the cast, the RA reused
                // its home between cast and head, and the fused head read
                // the clobbered register.
                let widening = matches!(
                    (from_ty, to_ty),
                    (
                        IrType::U8
                            | IrType::U16
                            | IrType::U32
                            | IrType::Ptr
                            | IrType::I8
                            | IrType::I16
                            | IrType::I32,
                        IrType::I64 | IrType::U64
                    )
                );
                if widening {
                    feeder_cast_src.insert(dest.0, src.clone());
                }
            }
        }
    }
    for chain in &chains.chains {
        // Tail dest born at the head point.
        for iv in &mut liveness.intervals {
            if iv.value_id == chain.tail_dest && chain.head_point < iv.start {
                iv.start = chain.head_point;
            }
        }
        for seg in &mut liveness.segments {
            if seg.value_id == chain.tail_dest && chain.head_point < seg.start {
                seg.start = chain.head_point;
            }
        }
        // Feeder sources live until the head point.
        for f in chain.feeders() {
            let Some(src) = feeder_cast_src.get(&f) else {
                continue;
            };
            let Operand::Value(sv) = src else { continue };
            for iv in &mut liveness.intervals {
                if iv.value_id == sv.0 && iv.end < chain.head_point {
                    iv.end = chain.head_point;
                }
            }
            for seg in &mut liveness.segments {
                if seg.value_id == sv.0 && seg.end < chain.head_point {
                    seg.end = chain.head_point;
                }
            }
        }
    }
    liveness
        .segments
        .sort_by(|a, b| (a.start, a.value_id).cmp(&(b.start, b.value_id)));
}

fn loop_weight(depth: u32) -> u64 {
    match depth {
        0 => 1,
        1 => 10,
        2 => 100,
        3 => 1_000,
        _ => 10_000,
    }
}

fn is_non_gpr_type(ty: &IrType, is_32bit: bool) -> bool {
    ty.is_float()
        || ty.is_long_double()
        || matches!(ty, IrType::I128 | IrType::U128)
        || (is_32bit && matches!(ty, IrType::I64 | IrType::U64))
}

fn propagate_coalesce_members(
    assignments: &mut FxHashMap<u32, PhysReg>,
    member_of: &FxHashMap<u32, u32>,
) {
    for (&member, &leader) in member_of {
        match assignments.get(&leader).copied() {
            Some(reg) => {
                assignments.insert(member, reg);
            }
            None => {
                assignments.remove(&member);
            }
        }
    }
}

fn evict_group(
    assignments: &mut FxHashMap<u32, PhysReg>,
    holders_by_reg: &mut FxHashMap<u8, Vec<u32>>,
    vid: u32,
    member_of: &FxHashMap<u32, u32>,
    groups: &FxHashMap<u32, Vec<u32>>,
) {
    let leader = member_of.get(&vid).copied().unwrap_or(vid);
    // `&[vid]` is a temporary that cannot outlive this statement; bind it so
    // the borrow checker sees a stable slice for the unwrap_or fallback.
    let solo = [vid];
    let members: &[u32] = groups.get(&leader).map(|m| m.as_slice()).unwrap_or(&solo);
    for &m in members {
        if let Some(reg) = assignments.remove(&m) {
            if let Some(h) = holders_by_reg.get_mut(&reg.0) {
                h.retain(|x| *x != m);
            }
        }
    }
}

/// Union-find over eligible `Copy dest, Value(src)`. Kept iff members'
/// intervals are pairwise disjoint. Representative is always an accepted member.
/// Re-weight coalesced group leaders by the group's total loop-weighted use
/// count. `build_live_ranges` keys its metadata by the leader's value id, so a
/// leader that is a ParamRef or an entry-block Copy carries only its own
/// single use and would be ranked below the hot loop temps it actually feeds
/// (through its members) — the parameters then lose their callee-saved home
/// in Phase 2c and round-trip through stack slots. The sum of the members'
/// `use_count` (already loop-depth-weighted) is the correct ranking weight.
fn bump_coalesce_group_priority(
    ranges: &mut [crate::backend::live_range::LiveRange],
    groups: &FxHashMap<u32, Vec<u32>>,
    use_count: &FxHashMap<u32, u64>,
) {
    if groups.is_empty() {
        return;
    }
    for r in ranges {
        if let Some(members) = groups.get(&r.value_id) {
            let total: u64 = members
                .iter()
                .map(|m| use_count.get(m).copied().unwrap_or(0))
                .sum();
            if total > r.priority {
                r.priority = total;
                r.calculate_spill_weight();
            }
        }
    }
}

/// GEP bases folded into Load/Store addressing are read at every folded access
/// point, but `build_live_ranges` only counts their direct operand uses (the
/// GEP instructions themselves), so a hot-loop base carries priority 1 while
/// its live interval spans the whole loop. The eviction scan then treats the
/// base as dead-past-its-one-use and evicts it, turning every folded access
/// into a reload of the base register (memcmp/adler32 spilled their pointer
/// params, +35/+40 bytes). Rank these values by the largest loop weight their
/// interval touches instead — the fold makes them live-and-read throughout.
fn apply_physical_reg_hints(
    ranges: &mut [crate::backend::live_range::LiveRange],
    hints: &FxHashMap<u32, PhysReg>,
) {
    for range in ranges {
        // Dataflow coalescing is stronger than an ABI preference: overwriting
        // `follow_value` would reintroduce producer/consumer copies.
        if range.follow_value.is_none() {
            range.reg_hint = hints.get(&range.value_id).copied();
        }
    }
}

fn collect_safe_folded_index_homes(
    func: &IrFunction,
    folded_index_uses: &FxHashMap<u32, Vec<u32>>,
) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::And,
                lhs,
                rhs,
                ty: IrType::I32 | IrType::U32,
            } = inst
            else {
                continue;
            };
            if !folded_index_uses.contains_key(&dest.0) {
                continue;
            }
            let mask = match (lhs, rhs) {
                (Operand::Const(c), _) | (_, Operand::Const(c)) => c.to_i64(),
                _ => None,
            };
            // Byte-table indexes are canonicalized by the mask itself and
            // have no Copy/phi materialization ambiguity. Broader hidden-index
            // homes remain blocked on RA-23 (phi_cfg_fuzz catches them).
            if mask.is_some_and(|value| (0..=255).contains(&value)) {
                result.insert(dest.0);
            }
        }
    }
    // RA-23 follow-up: fresh-arithmetic hidden indexes. A value whose def is
    // a pure BinOp (URem/Shl/Add/... — born fresh each iteration, never a
    // Copy/Phi web member) and whose SINGLE function-wide use feeds a folded
    // GEP's offset chain. vsprintf number()'s digit loop: the URem remainder
    // is the natural index of the digits load; keeping it eligible lets the
    // caller-saved phases home it in %edx (born there at the fused div) and
    // the load folds to `movsbl digits(,%edx),%eax` — GCC's shape, −5
    // instructions per iteration. Single-use ⇒ the value is the source of
    // no Copy and the dest of no Phi ⇒ no coalescing web ⇒ none of the RA-23
    // materialization ambiguity the narrow And-mask exception guards against;
    // the emitter still folds only when a physical home was actually
    // assigned, and an unhomed value keeps its accumulator-flow shape.
    let mut use_counts: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_counts.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_counts.entry(v.0).or_insert(0) += 1;
            }
        });
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp { dest, .. } = inst {
                if folded_index_uses.contains_key(&dest.0)
                    && use_counts.get(&dest.0).copied() == Some(1)
                {
                    result.insert(dest.0);
                }
            }
        }
    }
    result
}

fn bump_folded_index_priority(
    ranges: &mut [crate::backend::live_range::LiveRange],
    folded_index_uses: &FxHashMap<u32, Vec<u32>>,
    safe_homes: &FxHashSet<u32>,
) {
    if env_on("CCC_NO_INDEX_HOME") {
        return;
    }
    for range in ranges {
        if !safe_homes.contains(&range.value_id) {
            continue;
        }
        let Some(consumers) = folded_index_uses.get(&range.value_id) else {
            continue;
        };
        // The folded Load/Store reads the index outside the visible IR use
        // chain. Rank it by loop frequency and number of folded consumers so a
        // one-use mask/index does not lose its home to transient temporaries.
        // One indexed home removes the complete scale/address materialization
        // chain (typically Cast+Shl+LEA) in addition to the memory access. Give
        // it a 64x structural benefit multiplier, analogous to a multi-use hot
        // value rather than a one-use temporary.
        let weight = live_range::loop_depth_weight(range.loop_depth)
            .saturating_mul(consumers.len().max(1) as u64)
            .saturating_mul(64);
        if weight > range.priority {
            range.priority = weight;
            range.calculate_spill_weight();
        }
    }
}

fn bump_gep_base_priority(
    ranges: &mut [crate::backend::live_range::LiveRange],
    liveness: &LivenessResult,
) {
    if liveness.gep_base_values.is_empty() {
        return;
    }
    for r in ranges {
        if !liveness.gep_base_values.contains(&r.value_id) {
            continue;
        }
        // `loop_depth` is the max of the def block and use-block depths; a
        // base folded across an inner loop is read every iteration there.
        let weight = crate::backend::live_range::loop_depth_weight(r.loop_depth);
        let boosted = r.priority.max(weight);
        if boosted > r.priority {
            r.priority = boosted;
            r.calculate_spill_weight();
        }
    }
}

/// For each value with exactly one definition that is a `Load`, the loaded
/// type, paired with the value's operand-use count. Values with multiple defs
/// (phis) or non-load defs are absent. A sub-word load (I8/U8/I16/U16)
/// sign/zero-extends into its register (`movsbl`/`movzbl`/`movswl`/`movzwl`),
/// so a widening cast of that load is a bit-preserving no-op and may coalesce
/// exactly like a Copy.
fn unique_load_def_types(func: &IrFunction) -> FxHashMap<u32, (IrType, u32)> {
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut load_ty: FxHashMap<u32, IrType> = FxHashMap::default();
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
                if let Instruction::Load { ty, .. } = inst {
                    load_ty.insert(dest.0, *ty);
                }
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }
    load_ty.retain(|k, _| def_count.get(k).copied() == Some(1));
    load_ty
        .into_iter()
        .map(|(k, ty)| (k, (ty, use_count.get(&k).copied().unwrap_or(0))))
        .collect()
}

fn build_coalesce_groups(
    func: &IrFunction,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    eligible: &FxHashSet<u32>,
    param_ref_values: &FxHashSet<u32>,
) -> FxHashMap<u32, Vec<u32>> {
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    let load_def_types = unique_load_def_types(func);
    // (cast dest, cast src) edges where the cast is a bit-preserving no-op on
    // the register: same-width U32<->I32, or a widening of a sub-word load
    // whose register already holds the extended value. For these, dest and src
    // hold IDENTICAL values, so their live ranges may overlap and still share
    // a register (the cast emits nothing) — the strict non-overlap check that
    // guards general Copy coalescing does not apply.
    let mut same_value_edges: FxHashSet<(u32, u32)> = FxHashSet::default();
    // Phi-web detection: a Copy whose DEST feeds a Phi (loop-latch / switch-arm
    // state transport) or whose SRC is a Phi result moves a bit-IDENTICAL
    // value — the same SSA renaming argument as the no-op casts above, so the
    // edge may overlap and still share one home. This is what collapses
    // switch-state webs (cpucheck/cmdline `state` machines: N overlapping
    // copies of ONE C variable, each with its own slot) into a single home.
    let mut phi_dests: FxHashSet<u32> = FxHashSet::default();
    let mut phi_operands: FxHashSet<u32> = FxHashSet::default();
    // Phi-congruence classes (the CFG-aware lever): a Phi's dest and every
    // incoming value are the SAME source-level variable observed on mutually
    // exclusive control-flow paths — on any real execution exactly one
    // predecessor runs, so exactly one of them is live at the merge.  Linear
    // scan sees their intervals overlap (the linearisation spans blocks), but
    // they may safely share ONE home.  Union-find each phi's dest with its
    // incoming GPR values; the accept loop below treats members of one class
    // as same-value even when intervals overlap.  This is what collapses
    // switch-state / loop-carried webs (cpucheck & cmdline `state` machines:
    // N overlapping SSA copies of ONE C variable, each with its own slot)
    // onto a single register/slot.
    let mut web_parent: FxHashMap<u32, u32> = FxHashMap::default();
    fn wfind(wp: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
        let mut root = x;
        while wp.get(&root).copied() != Some(root) {
            root = wp.get(&root).copied().unwrap_or(root);
        }
        let mut c = x;
        while wp.get(&c).copied().is_some_and(|p| p != root) {
            let next = wp[&c];
            wp.insert(c, root);
            c = next;
        }
        root
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                phi_dests.insert(dest.0);
                for (op, _) in incoming {
                    if let Operand::Value(v) = op {
                        phi_operands.insert(v.0);
                        if eligible.contains(&dest.0) && eligible.contains(&v.0) {
                            web_parent.entry(dest.0).or_insert(dest.0);
                            web_parent.entry(v.0).or_insert(v.0);
                            let rd = wfind(&mut web_parent, dest.0);
                            let rv = wfind(&mut web_parent, v.0);
                            if rd != rv {
                                web_parent.insert(rv, rd);
                            }
                        }
                    }
                }
            }
        }
    }
    // Flatten to a direct value -> class-rep map for O(1) accept-loop lookup.
    let mut phi_web_class: FxHashMap<u32, u32> = FxHashMap::default();
    let web_keys: Vec<u32> = web_parent.keys().copied().collect();
    for v in web_keys {
        let rep = wfind(&mut web_parent, v);
        phi_web_class.insert(v, rep);
    }
    fn find(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
        let mut root = x;
        while parent.get(&root).copied() != Some(root) {
            root = parent.get(&root).copied().unwrap_or(root);
        }
        let mut c = x;
        while parent.get(&c).copied().is_some_and(|p| p != root) {
            let next = parent[&c];
            parent.insert(c, root);
            c = next;
        }
        root
    }

    // Copy-latch webs (phi-elim form): a Copy destination with 2+
    // definitions is a phi-elim latch. When the latch Copy{V <- s} sits
    // IMMEDIATELY after s's defining instruction, s's def consumed V's old
    // incarnation (it reads V) and s dies at the copy — s may share V's
    // home, so `flags |= X` lowers to `orl $X, %reg` instead of a
    // 3-instruction relay through a second register. Adjacency is the
    // soundness anchor: with anything between s's def and the copy, a use
    // of V's OLD incarnation could sit inside s's live range.
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut copy_dests: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
            }
            if let Instruction::Copy { dest, .. } = inst {
                copy_dests.insert(dest.0);
            }
            inst.for_each_used_value(|u| {
                *use_count.entry(u).or_insert(0) += 1;
            });
        }
        block
            .terminator
            .for_each_used_value(|u| *use_count.entry(u).or_insert(0) += 1);
    }
    let latch_dests: FxHashSet<u32> = copy_dests
        .into_iter()
        .filter(|v| def_count.get(v).copied().unwrap_or(0) > 1)
        .collect();
    let mut latch_same_value: FxHashSet<(u32, u32)> = FxHashSet::default();
    if !latch_dests.is_empty() {
        for block in &func.blocks {
            for w in block.instructions.windows(2) {
                if let (
                    prev,
                    Instruction::Copy {
                        dest: d,
                        src: Operand::Value(s),
                    },
                ) = (&w[0], &w[1])
                {
                    if latch_dests.contains(&d.0) {
                        if let Some(pd) = prev.dest() {
                            // The latch-same-value relaxation is only sound when
                            // the source `s` is single-use and therefore dies at
                            // this copy. If `s` has another use later (e.g. a
                            // loop-invariant like a bound operand that is re-used
                            // in the loop header), then `s` is live across the
                            // latch copy and holds a DIFFERENT value than `d`
                            // after `d` is redefined — coalescing them lets the
                            // latch redefinition corrupt `s` (the HUF rankVal
                            // loop's `minBits` operand is exactly this shape and
                            // previously turned the outer-loop bound into a
                            // `consumed`-dependent value, running the loop one
                            // iteration short and leaving the HUF DTable half
                            // built, which made zstd's 4X2 HUF fast loop spin
                            // forever on a zeroed table slot). Requiring
                            // use_count == 1 restores the "s dies at the copy"
                            // invariant.
                            if pd.0 == s.0 && use_count.get(&s.0).copied().unwrap_or(0) == 1 {
                                latch_same_value.insert((d.0, s.0));
                            }
                        }
                    }
                }
            }
        }
    }

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } = inst
            {
                let (d, s) = (dest.0, v.0);
                // Do NOT coalesce a ParamRef with its loop-carried Copy: the
                // param's ABI home dies at the Copy, while the loop copy wants
                // its own caller-saved register. Merging them forces one
                // register to hold the value across the whole loop, stealing a
                // callee-saved register from the hot-loop overflow temps
                // (adler32/memcmp spilt the parameters; the pre-rework
                // allocator never coalesced them).
                if d != s
                    && eligible.contains(&d)
                    && eligible.contains(&s)
                    && !param_ref_values.contains(&s)
                {
                    parent.entry(d).or_insert(d);
                    parent.entry(s).or_insert(s);
                    let rd = find(&mut parent, d);
                    let rs = find(&mut parent, s);
                    if rd != rs {
                        parent.insert(rs, rd);
                    }
                    // Phi-transport copies move a bit-identical value (the
                    // dest feeds a Phi, or the src IS a Phi result), so the
                    // edge may overlap and still share one home — exactly the
                    // no-op-cast relaxation. This collapses switch-state and
                    // loop-carried webs onto a single register/slot.
                    // Adjacent copy-latch edges are the same story in the
                    // phi-elim form (see latch_same_value above).
                    if phi_operands.contains(&d)
                        || phi_dests.contains(&s)
                        || latch_same_value.contains(&(d, s))
                    {
                        same_value_edges.insert((d, s));
                    }
                }
            }
            // A same-width 32-bit int cast (U32<->I32) is a bit-preserving
            // no-op on 32-bit targets: the register contents are identical, so
            // dest and src may share a register exactly like a Copy. Without
            // this, `cpu_vendor[0] == 'Genu'` (a u32 global compared as I32)
            // materializes as `movl sym,%edx; movl %edx,%ebx; cmpl ...,%ebx`
            // — the no-op cast relay that dominates global-load compare sites.
            if let Instruction::Cast {
                dest,
                src: Operand::Value(v),
                from_ty,
                to_ty,
            } = inst
            {
                let noop32 = crate::common::types::target_is_32bit()
                    && matches!(
                        (from_ty, to_ty),
                        (IrType::I32, IrType::U32)
                            | (IrType::U32, IrType::I32)
                            | (IrType::I32, IrType::I32)
                            | (IrType::U32, IrType::U32)
                    );
                // A widening cast of a sub-word load is a no-op: the load
                // already sign/zero-extends into its register (`movsbl
                // (%mem),%r`), so `(I32)(I8)*p` may share the load's register.
                // Fixes strchr's `movsbl (%ebx),%esi; movl %esi,%eax; movl
                // %eax,%edi; cmpl %ebp,%edi` relay: the cast and the load
                // collapse, leaving `movsbl (%ebx),%esi; cmpl %ebp,%esi`.
                //
                // Gate on `uses >= 2`: when the load feeds ONLY the cast
                // (single use), the load was likely to spill anyway, and
                // merging it into the cast drags the cast's register down
                // with it (skip_atoi: the cast lost %edi and the whole pair
                // spilled). When the load has another use, it needs a
                // register regardless, so coalescing is a pure win.
                let (_, load_uses) = load_def_types
                    .get(&v.0)
                    .copied()
                    .unwrap_or((IrType::I32, 0));
                let widen_from_load = crate::common::types::target_is_32bit()
                    && matches!(from_ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16)
                    && matches!(to_ty, IrType::I32 | IrType::U32)
                    && load_def_types.get(&v.0).map(|&(ty, _)| ty) == Some(*from_ty)
                    && load_uses >= 2;
                let (d, s) = (dest.0, v.0);
                // ParamRef sources: the loop-carried-Copy rationale (the
                // param's ABI home dies at a redefining Copy while the loop
                // copy wants its own register) does NOT apply to a no-op
                // same-width cast — the cast is a pure value-preserving view
                // of an UNCHANGED parameter, so sharing the param's home is
                // exactly the intended shape (number()'s `(u32)base` reading
                // %edi directly at both div sites). Keep the exclusion for
                // the sub-word widen case: there the source is a load whose
                // register economics the ParamRef guard was protecting.
                let src_ok = if noop32 {
                    true
                } else {
                    !param_ref_values.contains(&s)
                };
                if (noop32 || widen_from_load)
                    && d != s
                    && eligible.contains(&d)
                    && eligible.contains(&s)
                    && src_ok
                {
                    parent.entry(d).or_insert(d);
                    parent.entry(s).or_insert(s);
                    let rd = find(&mut parent, d);
                    let rs = find(&mut parent, s);
                    if rd != rs {
                        parent.insert(rs, rd);
                        same_value_edges.insert((d, s));
                    }
                }
            }
        }
    }
    if parent.is_empty() {
        return FxHashMap::default();
    }

    let mut groups: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let vids: Vec<u32> = parent.keys().copied().collect();
    for vid in vids {
        let leader = find(&mut parent, vid);
        groups.entry(leader).or_default().push(vid);
    }

    let mut result: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (leader, mut members) in groups {
        members.sort_by_key(|&m| iv_map.get(&m).map(|&(s, _)| s).unwrap_or(0));
        let mut accepted: Vec<u32> = Vec::new();
        for m in members {
            let ok = match iv_map.get(&m) {
                None => true,
                Some(&mi) => accepted.iter().all(|a| {
                    iv_map.get(a).is_none_or(|&ai| {
                        !intervals_overlap(mi, ai)
                            // A same-value cast edge (no-op U32<->I32 or a
                            // widening of a sub-word load) may overlap: dest
                            // and src hold identical register contents, so
                            // sharing the register is sound and the cast
                            // emits nothing.
                            || same_value_edges.contains(&(m, *a))
                            || same_value_edges.contains(&(*a, m))
                            // Phi-congruence: dest/incomings of a phi are one
                            // variable on exclusive CFG paths (see the
                            // web_parent construction above).
                            || phi_web_class.get(&m).zip(phi_web_class.get(a))
                                .is_some_and(|(cm, ca)| cm == ca)
                    })
                }),
            };
            if ok {
                accepted.push(m);
            }
        }
        if accepted.len() > 1 {
            let rep = if accepted.contains(&leader) {
                leader
            } else {
                accepted[0]
            };
            result.insert(rep, accepted);
        }
    }

    if env_on("CCC_DEBUG_COALESCE") {
        eprintln!(
            "[COALESCE] fn={} groups={} members={}",
            func.name,
            result.len(),
            result.values().map(|m| m.len()).sum::<usize>()
        );
        if std::env::var_os("CCC_DEBUG_COALESCE_MEMBERS").is_some() {
            let mut rows: Vec<(&u32, &Vec<u32>)> = result.iter().collect();
            rows.sort_unstable();
            for (leader, members) in rows {
                eprintln!("[COALESCE]   leader=v{} members={:?}", leader, members);
            }
        }
    }
    result
}

fn collect_gpr_scan_intervals(
    liveness: &LivenessResult,
    eligible: &FxHashSet<u32>,
    merged_of: &FxHashMap<u32, LiveInterval>,
    member_of: &FxHashMap<u32, u32>,
) -> Vec<LiveInterval> {
    let mut out = Vec::new();
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for iv in &liveness.intervals {
        if member_of.contains_key(&iv.value_id) {
            continue;
        }
        if let Some(&merged) = merged_of.get(&iv.value_id) {
            if seen.insert(merged.value_id) {
                out.push(merged);
            }
            continue;
        }
        if eligible.contains(&iv.value_id) && iv.end > iv.start && seen.insert(iv.value_id) {
            out.push(*iv);
        }
    }
    // Leader may have a degenerate raw interval; members still have range.
    for (&leader, merged) in merged_of {
        if eligible.contains(&leader) && merged.end > merged.start && seen.insert(leader) {
            out.push(*merged);
        }
    }
    out
}

fn unite_map(parent: &mut FxHashMap<u32, u32>, a: u32, b: u32) {
    fn find_r(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
        let mut root = x;
        while parent.get(&root).copied().is_some_and(|p| p != root) {
            root = parent[&root];
        }
        let mut c = x;
        while parent.get(&c).copied().is_some_and(|p| p != root) {
            let next = parent[&c];
            parent.insert(c, root);
            c = next;
        }
        root
    }
    parent.entry(a).or_insert(a);
    parent.entry(b).or_insert(b);
    let ra = find_r(parent, a);
    let rb = find_r(parent, b);
    if ra != rb {
        parent.insert(rb, ra);
    }
}

/// Class-union overlap scan (the verifier's model, as a query): returns
/// (reg, class_a, class_b, overlap_start, overlap_end) for every pair of
/// DISTINCT classes whose register's merged class coverage overlaps.
fn find_overlapping_classes(
    liveness: &LivenessResult,
    assignments: &FxHashMap<u32, PhysReg>,
    parent: &FxHashMap<u32, u32>,
) -> Vec<(u8, u32, u32, u32, u32)> {
    fn find_ro(parent: &FxHashMap<u32, u32>, x: u32) -> u32 {
        let mut root = x;
        while let Some(&p) = parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        root
    }
    let mut group_segments: FxHashMap<(u8, u32), Vec<(u32, u32)>> = FxHashMap::default();
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for segment in &liveness.segments {
        let Some(&reg) = assignments.get(&segment.value_id) else {
            continue;
        };
        segmented.insert(segment.value_id);
        let rep = find_ro(parent, segment.value_id);
        group_segments
            .entry((reg.0, rep))
            .or_default()
            .push((segment.start, segment.end));
    }
    for interval in &liveness.intervals {
        if segmented.contains(&interval.value_id) {
            continue;
        }
        let Some(&reg) = assignments.get(&interval.value_id) else {
            continue;
        };
        let rep = find_ro(parent, interval.value_id);
        group_segments
            .entry((reg.0, rep))
            .or_default()
            .push((interval.start, interval.end));
    }
    let mut by_reg: FxHashMap<u8, Vec<(u32, u32, u32)>> = FxHashMap::default();
    for ((reg, rep), pieces) in &mut group_segments {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_segment_union(pieces, &source);
        by_reg
            .entry(*reg)
            .or_default()
            .extend(pieces.iter().map(|&(s, e)| (s, e, *rep)));
    }
    let mut out = Vec::new();
    for (&reg, events) in by_reg.iter_mut() {
        events.sort_unstable();
        let mut previous: Option<(u32, u32, u32)> = None;
        for &(start, end, rep) in events.iter() {
            if let Some((_, prev_end, prev_rep)) = previous {
                if start < prev_end && rep != prev_rep {
                    out.push((reg, prev_rep, rep, start, prev_end.min(end)));
                }
            }
            if previous.is_none_or(|(_, pe, _)| end > pe) {
                previous = Some((start, end, rep));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub fn allocate_registers(func: &IrFunction, config: &RegAllocConfig) -> RegAllocResult {
    let has_builtin_setjmp = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::BuiltinSetjmp,
                    ..
                }
            )
        })
    });
    // __builtin_apply performs an indirect call from inside the intrinsic and
    // clobbers every argument/caller-saved register plus rax/rdx/xmm0 (resp.
    // eax/edx on i686).  Register-homed values would be silently invalidated,
    // so such functions fall back to pure memory allocation.
    let has_builtin_apply = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::DoBuiltinApply,
                    ..
                }
            )
        })
    });
    if has_builtin_setjmp
        || has_builtin_apply
        || (config.available_regs.is_empty() && config.caller_saved_regs.is_empty())
    {
        return RegAllocResult {
            assignments: FxHashMap::default(),
            accumulator_assignments: if has_builtin_setjmp {
                Vec::new()
            } else {
                analyze_accumulator_assignments(func, config.accumulator_policy)
            },
            used_regs: Vec::new(),
            caller_save_spans: FxHashMap::default(),
            liveness: None,
        };
    }

    let is_32bit = crate::common::types::target_is_32bit();
    let mut liveness = compute_live_intervals(func);
    // DivRem pair tails are physically born at their head's dual-store;
    // extend their intervals before ANY interval map is derived. Without
    // this, homes/slots in the head..tail window get double-assigned.
    patch_divrem_tail_intervals(func, &mut liveness);
    // Mul-acc chain tails are born at their head's fused store, and the
    // virtual feeder sources live until the head reads them — same contract.
    patch_mulacc_intervals(func, &mut liveness);
    // Extend live intervals for backend-folded index consumers BEFORE any
    // interval map is derived (see RegAllocConfig::folded_index_uses).
    if !config.folded_index_uses.is_empty() && !env_on("CCC_NO_FOLDED_INDEX_LIVENESS") {
        // value_id -> (start, end) from the consumer GEP-dest intervals. The
        // dest's END is the access that re-reads the operand RA-invisibly
        // (SIB load/store, replayed cmp). dest.start is the GEP / Cmp itself
        // — NOT always the last IR-visible read of `idx`: `resolve_index`
        // peels Cast/Shl/Mul/Add, so a peeled index's last IR use is the
        // widening Cast sitting BEFORE the GEP (sqlite3 vdbeChangeP4Full).
        let mut bounds_of: FxHashMap<u32, (u32, u32)> = FxHashMap::default();
        for iv in &liveness.intervals {
            bounds_of.insert(iv.value_id, (iv.start, iv.end));
        }
        // Multi-def (phi-elim latch) values: the only shape whose folded
        // consumer can sit inside a liveness HOLE before the final segment,
        // where a tail-only stretch is a silent no-op. For single-def values
        // the historical LAST-segment stretch is kept: merging the consumer
        // spans into the union moves the coverage boundary anchors
        // (first/last live unit) that the segment scan's die-at-birth
        // permission is computed against, and for values whose hole is not
        // real liveness (no redefinition between producer and access) the
        // moved anchors let a boundary touch through at a NON-boundary
        // position — under-constraining exactly where the merge meant to
        // over-constrain (preboot-ZSTD: every pattern failed with
        // "ZSTD-compressed data is corrupt" until the merge was gated back
        // to multi-def latches).
        let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    *def_count.entry(d.0).or_insert(0) += 1;
                }
            }
        }
        let multi_def: FxHashSet<u32> = def_count
            .iter()
            .filter(|(_, &c)| c > 1)
            .map(|(&v, _)| v)
            .collect();
        for (idx, dests) in &config.folded_index_uses {
            let mut new_end: Option<u32> = None;
            // Real-liveness ranges the operand must survive: one per
            // consumer. dest.start is the GEP, but a peeled index's last
            // IR use is typically a widening Cast / scale BEFORE the GEP
            // (the GEP reads orig_offset). Starting required at dest.start
            // then misses every call between that last IR use and the GEP:
            // sqlite3 vdbeChangeP4Full inlined sqlite3DbStrNDup homes the
            // multi-def I32 Copy of `n` in %r10, memcpy clobbers it, then
            // ensure_sib_index_form movslqs the stale register for p[n]=0.
            // required starts at the last IR-visible segment end when the
            // index is not live at dest.start (fills Cast→Store including
            // the call). A covering segment keeps [GEP, Store].
            let mut required: Vec<(u32, u32)> = Vec::new();
            // Fat interval end is the block envelope (join block runs through
            // memcpy+GEP+Store), so it is NOT the last IR use. Use hole-aware
            // segments: if idx is not live at dest.start, start required at
            // the preceding segment end (the peeled Cast) so calls in the
            // hole are covered. A covering segment keeps historical
            // [GEP, Store] (vsprintf digit latch).
            let idx_segs: Vec<(u32, u32)> = liveness
                .segments
                .iter()
                .filter(|seg| seg.value_id == *idx)
                .map(|seg| (seg.start, seg.end))
                .collect();
            for d in dests {
                if let Some(&(s, e)) = bounds_of.get(d) {
                    new_end = Some(new_end.map_or(e, |x: u32| x.max(e)));
                    if s < e {
                        let covering = idx_segs.iter().any(|&(ss, ee)| ss <= s && s < ee);
                        let req_s = if covering {
                            s
                        } else {
                            idx_segs
                                .iter()
                                .map(|&(_, ee)| ee)
                                .filter(|&ee| ee <= s)
                                .max()
                                .unwrap_or(s)
                        };
                        if req_s < e {
                            required.push((req_s, e));
                        }
                    }
                }
            }
            if let Some(e) = new_end {
                for iv in &mut liveness.intervals {
                    if iv.value_id == *idx && iv.end < e {
                        iv.end = e;
                    }
                }
                // Hole-aware segments. Stretching only the LAST segment is a
                // silent NO-OP for multi-def latch values: their fat envelope
                // already ends past the access (the backedge copy), so both
                // `iv.end < e` and `last_seg.end < e` are false while a live
                // HOLE sits exactly on the access — the IR's last read of the
                // operand is the folded-away GEP, and liveness legitimately
                // dies there. The scan's hole-aware interference then reused
                // the register across the access: vsprintf number()'s
                // `tmp[i++] = digit` wrote tmp[digit] because the digit's
                // zext landed in i's register between the GEP and the store.
                // The operand IS read at the producer position (its segment
                // covers [.., producer]) and re-read at the access, so
                // [producer, access] is real liveness for multi-def latches:
                // merge it into the segment union. Single-def values keep
                // the historical tail stretch (see the multi_def note above
                // for why the unconditional merge is NOT sound).
                let mut pieces: Vec<(u32, u32)> = Vec::new();
                let mut has_segments = false;
                liveness.segments.retain(|seg| {
                    if seg.value_id != *idx {
                        return true;
                    }
                    has_segments = true;
                    pieces.push((seg.start, seg.end));
                    false
                });
                if has_segments {
                    if multi_def.contains(idx) && !required.is_empty() {
                        required.sort_unstable();
                        pieces.sort_unstable();
                        let mut merged: Vec<(u32, u32)> = Vec::new();
                        insert_segment_union(&mut merged, &pieces);
                        insert_segment_union(&mut merged, &required);
                        pieces = merged;
                    } else {
                        // Historical tail stretch (single-def shapes).
                        if let Some(last) = pieces.last_mut() {
                            if last.1 < e {
                                last.1 = e;
                            }
                        }
                        pieces.sort_unstable();
                    }
                    for &(s, e2) in &pieces {
                        liveness
                            .segments
                            .push(crate::backend::liveness::LiveInterval {
                                value_id: *idx,
                                start: s,
                                end: e2,
                            });
                    }
                }
                // Values without segment data keep the fat-envelope
                // extension above (fail-closed: the scan and the verifier
                // both fall back to the envelope for them).
            }
        }
    }
    let iv_map = interval_map(&liveness);
    let call_points = &liveness.call_points;

    let arm_fp_pool =
        config.xmm_regs.first().is_some_and(|r| r.0 == 40) && !env_on("CCC_NO_VECREG");
    // XMM2 is deliberately removed when an intrinsic uses it as implicit
    // scratch, so the safe x86 pool may start at PhysReg(21) (XMM3). Do not
    // mistake that quarantine for a non-x86 pool and disable SIMD allocation.
    let x86_fp_pool = config
        .xmm_regs
        .first()
        .is_some_and(|r| (20..=33).contains(&r.0));
    let non_gpr_values = collect_non_gpr_values(func, is_32bit);
    // Dest values of indexed-fold GEPs: single-use address temporaries whose
    // register home is worthless when the SIB fold fires (they emit nothing)
    // and no better than a slot when it does not. Denied %ecx/%edx homes on
    // the i686 caller-saved pool so the loop-carried INDEX can claim the
    // register instead (Phase 2/2d/2h filters; see
    // collect_i686_scratch_denials for the full denial policy: indexed-GEP
    // dests, their GlobalAddr bases and load dests, and div quotients).
    let scratch_denied = if is_32bit {
        crate::backend::generation::collect_i686_scratch_denials(func)
    } else {
        FxHashSet::default()
    };
    // Values consumed as call arguments: their last read happens in the call's
    // argument staging, which writes the ABI arg registers in order. A home in
    // one of those registers (rdi/rsi/rdx/r8/r9 on x86-64) is clobbered by an
    // earlier argument before this value is read. Exclude arg registers from
    // the Phase-2 pool for exactly these values (see RegAllocConfig::call_arg_regs).
    let (later_arg_values, indirect_arg_values) = collect_call_arg_values(func);

    let block_loop_weight: Vec<u64> = liveness
        .block_loop_depth
        .iter()
        .map(|&d| loop_weight(d))
        .collect();

    let mut use_count: FxHashMap<u32, u64> = FxHashMap::default();
    // Hottest *use-site* loop depth per value. Unlike the def block, a
    // preheader-defined IV is used inside the loop — the canonical
    // loop-carried shape. Used by `hot_loop_home` (Phase-1 candidacy).
    let mut use_loop_depth: FxHashMap<u32, u32> = FxHashMap::default();
    let mut eligible: FxHashSet<u32> = FxHashSet::default();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let weight = block_loop_weight.get(block_idx).copied().unwrap_or(1);
        let depth = liveness
            .block_loop_depth
            .get(block_idx)
            .copied()
            .unwrap_or(0);
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. } | Instruction::UnaryOp { dest, ty, .. } => {
                    if !is_non_gpr_type(ty, is_32bit) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::Cmp { dest, .. } => {
                    eligible.insert(dest.0);
                }
                Instruction::Cast {
                    dest,
                    to_ty,
                    from_ty,
                    ..
                } => {
                    if !is_non_gpr_type(to_ty, is_32bit) && !is_non_gpr_type(from_ty, is_32bit) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    if !is_non_gpr_type(ty, is_32bit) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::GetElementPtr { dest, .. }
                | Instruction::GlobalAddr { dest, .. }
                | Instruction::LabelAddr { dest, .. }
                | Instruction::GetStaticChain { dest } => {
                    eligible.insert(dest.0);
                }
                Instruction::Copy { dest, .. } => {
                    if !non_gpr_values.contains(&dest.0) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        if !is_non_gpr_type(&info.return_type, is_32bit) {
                            eligible.insert(dest.0);
                        }
                    }
                }
                Instruction::Select { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. } => {
                    if !is_non_gpr_type(ty, is_32bit) {
                        eligible.insert(dest.0);
                    }
                }
                // Session 28: PHI destinations are real values with real live
                // intervals (loop-carried state: switch machines, running
                // pointers).  Without a register home every use reloads them
                // from a stack slot — the dominant slot traffic in the boot
                // corpus (cmdline_find_option's `state` machine: gcc keeps it
                // in a register across the whole loop).  The phi-coalesce
                // machinery propagates the assigned register to the backedge
                // sources (apply_phi_coalesce_assignments expects the phi
                // dest to hold one), so this composes with the existing
                // coalesce contract instead of bypassing it.
                _ => {}
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += weight;
                    let entry = use_loop_depth.entry(v.0).or_insert(0);
                    *entry = (*entry).max(depth);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                *use_count.entry(v.0).or_insert(0) += weight;
                let entry = use_loop_depth.entry(v.0).or_insert(0);
                *entry = (*entry).max(depth);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += weight;
                let entry = use_loop_depth.entry(v.0).or_insert(0);
                *entry = (*entry).max(depth);
            }
        });
    }

    remove_ineligible_operands(func, &mut eligible, config);
    let safe_folded_index_homes = collect_safe_folded_index_homes(func, &config.folded_index_uses);
    for v in &config.never_materialized {
        // A value that is immediately consumed by a Cast/Copy may also be the
        // natural index of a later backend-folded memory access. That hidden
        // use is absent from the immediate-consumer analysis: denying a home
        // forces the backend to rematerialize shift+LEA+load and prevents SIB/
        // indexed addressing. Keep it eligible; folded-index liveness extends
        // the value to the actual access and the emitter folds only when a
        // physical home was assigned.
        if env_on("CCC_NO_INDEX_HOME") || !safe_folded_index_homes.contains(v) {
            eligible.remove(v);
        }
    }

    let x86_ordered_param_copies = !is_32bit
        && config.available_regs.iter().any(|r| r.0 == 1)
        && config.caller_saved_regs.iter().any(|r| r.0 == 10)
        && x86_param_caller_homes_safe(func);
    // riscv64: same lever for the a0–a7 pool.  Param homes are pinned to
    // their incoming registers by the ABI hints, so the "ordered copies"
    // degenerate to zero instructions per ParamRef.
    let riscv_ordered_param_copies = config.available_regs.iter().any(|r| r.0 == 11)
        && config.caller_saved_regs.iter().any(|r| r.0 == 12)
        && riscv_param_caller_homes_safe(func);
    let ordered_param_homes = x86_ordered_param_copies || riscv_ordered_param_copies;
    let mut param_ref_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef { dest, .. } = inst {
                param_ref_values.insert(dest.0);
            }
        }
    }

    if x86_fp_pool {
        exclude_every_third_mul_temp(func, &mut eligible);
    }

    let all_phi_pairs = detect_phi_coalesce_groups(func, &liveness);
    let mut phi_coalesce: Vec<PhiCoalesceCandidate> = Vec::new();
    if !env_on("CCC_NO_PHI_COALESCE") {
        let mut seen_dest: FxHashSet<u32> = FxHashSet::default();
        for cand in &all_phi_pairs {
            if seen_dest.insert(cand.phi_dest) {
                phi_coalesce.push(*cand);
            }
        }
    }
    // A source defined *before* a caller-saved clobber in the same block
    // cannot inherit the phi dest's register unless that register is
    // callee-saved. Keep those sources eligible so Phase 1 can give them
    // their own callee-saved home (or they spill). Removing them here was
    // how `++i` before `strtol` inherited `%edi` and the latch read garbage
    // (zlib-ng minideflate / loop_iv_across_call).
    for candidate in &phi_coalesce {
        if !phi_window_clobbers_caller_saved(func, candidate) {
            eligible.remove(&candidate.backedge_src);
        }
    }

    let coalesce_groups: FxHashMap<u32, Vec<u32>> = if !env_on("CCC_NO_COALESCE") {
        build_coalesce_groups(func, &iv_map, &eligible, &param_ref_values)
    } else {
        FxHashMap::default()
    };
    let mut coalesce_member_of: FxHashMap<u32, u32> = FxHashMap::default();
    let mut merged_of: FxHashMap<u32, LiveInterval> = FxHashMap::default();
    for (leader, members) in &coalesce_groups {
        let mut start = u32::MAX;
        let mut end = 0u32;
        for &m in members {
            if let Some(&(s, e)) = iv_map.get(&m) {
                start = start.min(s);
                end = end.max(e);
            }
            if m != *leader {
                coalesce_member_of.insert(m, *leader);
            }
        }
        if start < end {
            merged_of.insert(
                *leader,
                LiveInterval {
                    value_id: *leader,
                    start,
                    end,
                },
            );
        }
    }

    // A coalesce group containing a ParamRef must not take a caller-saved home
    // in Phase 2: the param's range spans the whole function and would evict
    // the hot loop's caller-saved temps. The baseline kept multi-block params
    // in callee-saved registers (Phase 2c); with coalescing ON the param is
    // merged into a group whose leader is a loop-carried copy, and the merged
    // leader interval is what the scan sees. Propagate the restriction to the
    // group leader so the whole group is excluded from the caller-saved pool.
    let mut param_restricted: FxHashSet<u32> = param_ref_values.clone();
    for (leader, members) in &coalesce_groups {
        if members.iter().any(|m| param_ref_values.contains(m)) {
            param_restricted.insert(*leader);
        }
    }
    // riscv entry-prefix write guard: a value BORN before a still-unexecuted
    // ParamRef must never take a caller-saved (a-reg) home — its defining
    // instruction would write the incoming argument register out from under
    // the not-yet-run ParamRef (GetStaticChain's chain value homed in a0
    // destroyed the incoming parameter before its ParamRef ran: the nested
    // `x == lim` compare then read the chain twice and returned the wrong
    // arm).  The x86 path never saw this because its entry prefix is
    // ParamRef-only by construction; the riscv entry prefix interleaves
    // GetStaticChain/Alloca defs, so the guard is computed from the actual
    // instruction order.  Params themselves are exempt (their homes are the
    // hinted incoming registers, written by nobody but the caller).
    let riscv_entry_guard: FxHashSet<u32> = if config.available_regs.iter().any(|r| r.0 == 11)
        && config.caller_saved_regs.iter().any(|r| r.0 == 12)
    {
        let mut guard = FxHashSet::default();
        if let Some(block0) = func.blocks.first() {
            let mut later_params: usize = 0;
            for inst in block0.instructions.iter().rev() {
                if let Instruction::ParamRef { dest, .. } = inst {
                    later_params += 1;
                    let _ = dest;
                } else if later_params > 0 {
                    if let Some(d) = inst.dest() {
                        guard.insert(d.0);
                    }
                }
            }
        }
        guard
    } else {
        FxHashSet::default()
    };

    // Hole-aware call spanning (the "80% of LLVM's split" win): a value needs
    // a callee-saved home only when a call point falls INSIDE one of its live
    // segments (and after its def). A call strictly between two segments of a
    // diamond (in the gap) is on the dead arm and can never reach a use, so a
    // caller-saved home is sound there. A call AT a segment boundary is NOT in
    // a gap: a loop re-entry segment (the `.Lstr0` format string used every
    // iteration) starts exactly at the call that re-clobbers its register, so
    // `seg.start <= cp < seg.end` must be inclusive on the left. `cp > def`
    // keeps a value born at its own call (retval) non-spanning. Coalesced
    // members attribute their segments to the group leader so a call spanned
    // by the merged interval (but not by any member alone) still forces a
    // callee-saved home — the merged leader interval is what the scan sees.
    // `segments` covers non-alloca SSA values only; the synthetic alloca
    // vector intervals keep the fat `spans_any_call` check.
    let call_spanning: FxHashSet<u32> = {
        let mut acc: FxHashMap<u32, Vec<(u32, u32, u32)>> = FxHashMap::default();
        for seg in &liveness.segments {
            let def = iv_map
                .get(&seg.value_id)
                .map(|&(s, _)| s)
                .unwrap_or(seg.start);
            let owner = coalesce_member_of
                .get(&seg.value_id)
                .copied()
                .unwrap_or(seg.value_id);
            acc.entry(owner)
                .or_default()
                .push((seg.start, seg.end, def));
        }
        let mut set = FxHashSet::default();
        for (owner, entries) in &acc {
            for &(s, e, def) in entries {
                let mut idx = call_points.partition_point(|&cp| cp < s);
                while idx < call_points.len() && call_points[idx] < e {
                    if call_points[idx] > def {
                        set.insert(*owner);
                        break;
                    }
                    idx += 1;
                }
                if set.contains(owner) {
                    break;
                }
            }
        }
        set
    };

    let has_nonlocal_control = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| {
            matches!(inst, Instruction::NonlocalGotoSave { .. })
                || crate::backend::liveness::is_returns_twice_call(inst)
        })
    });
    if has_nonlocal_control {
        for &vid in &call_spanning {
            eligible.remove(&vid);
        }
        for (&member, &owner) in &coalesce_member_of {
            if call_spanning.contains(&owner) {
                eligible.remove(&member);
            }
        }
    }

    let scan_ivs =
        collect_gpr_scan_intervals(&liveness, &eligible, &merged_of, &coalesce_member_of);
    let build_gpr_ranges = |intervals: &[LiveInterval]| {
        let mut ranges = live_range::build_live_ranges(intervals, &liveness.block_loop_depth, func);
        apply_physical_reg_hints(&mut ranges, &config.reg_hints);
        bump_folded_index_priority(
            &mut ranges,
            &config.folded_index_uses,
            &safe_folded_index_homes,
        );
        // RA-05: hole-aware coverage for the scan's interference tests.
        // Values without segment data keep their fat semantics.
        attach_scan_segments(&mut ranges, &liveness, &coalesce_member_of);
        ranges
    };

    // Phase 1: callee-saved for values live across a call. GlobalAddr /
    // LabelAddr addresses are NOT excluded: codegen has no rematerialisation
    // path for them (the GlobalAddr instruction is emitted once and later uses
    // reload from the value's home), so excluding them from Phase 1 merely
    // turns a callee-saved home into a stack slot reloaded on every use
    // (nbody's `bodies` base: +274 bytes). Matches the pre-rework allocator.
    //
    // Session 28: HOT LOOP-CARRIED values join Phase 1 even without a call.
    // A long-lived value used on every loop iteration pays a slot reload on
    // EVERY use when spilled — the dominant slot traffic of the boot corpus
    // (cmdline_find_option's state machine, uses=71/81 loop-weighted, all
    // slotted while span-1 temps took caller-saved registers).  Giving them
    // callee-saved homes is exactly what gcc does on the same 6-register
    // budget.  Candidates: heavily used, long-lived, and *used inside a
    // loop* (short temps are Phase-2 fodder, not loop state).
    //
    // Session 41 fix: candidacy keys on the hottest USE-SITE loop depth,
    // not the def block. Loop-carried values are canonically defined in
    // the preheader (def block depth 0) and consumed inside the loop, so
    // the old def-block check rejected every one of them — gzip
    // `longest_match` then spilled `scan`/`best`/`cur_match`/`len` to the
    // stack while dead entry temps held registers.
    // Call-free leaf: a callee-saved home is never *required*, and every
    // one taken costs a push/pop pair. Let the hot loop values compete for
    // the volatile pool in Phase 2 first; Phase 2c still hands the overflow
    // a callee-saved home (see `RegAllocConfig::leaf_caller_saved_homes`).
    let leaf_prefers_caller_saved = config.leaf_caller_saved_homes
        && call_points.is_empty()
        && !env_on("CCC_NO_LEAF_CALLER_HOME");
    let hot_loop_home = |iv: &LiveInterval| -> bool {
        if env_on("CCC_NO_HOT_LOOP") || leaf_prefers_caller_saved {
            return false;
        }
        if call_spanning.contains(&iv.value_id) {
            return false;
        }
        if iv.end.saturating_sub(iv.start) < 10 {
            return false;
        }
        // Use pressure: the value's own uses, or — for a coalesce-group
        // leader (phi webs) — the group total.  The leader's raw count only
        // reflects the phi's own incoming edges (cmdline's state machine:
        // leader shows 10, the web totals 71+).
        let mut uc = use_count.get(&iv.value_id).copied().unwrap_or(0);
        if let Some(members) = coalesce_groups.get(&iv.value_id) {
            let total: u64 = members
                .iter()
                .map(|m| use_count.get(m).copied().unwrap_or(0))
                .sum();
            uc = uc.max(total);
        }
        if uc < 12 {
            return false;
        }
        // At least one use inside a loop block.
        use_loop_depth.get(&iv.value_id).copied().unwrap_or(0) >= 1
    };
    let phase1_intervals: Vec<LiveInterval> = scan_ivs
        .iter()
        .copied()
        .filter(|iv| call_spanning.contains(&iv.value_id) || hot_loop_home(iv))
        .collect();
    let mut phase1_ranges = build_gpr_ranges(&phase1_intervals);
    bump_coalesce_group_priority(&mut phase1_ranges, &coalesce_groups, &use_count);
    bump_gep_base_priority(&mut phase1_ranges, &liveness);
    let mut allocator = LinearScanAllocator::new(phase1_ranges, config.available_regs.clone());
    allocator.run();
    let mut assignments = allocator.assignments;
    if env_on("CCC_DEBUG_RA_PHASES") {
        let mut ids: Vec<u32> = assignments.keys().copied().collect();
        ids.sort_unstable();
        eprintln!(
            "[RA-P1] fn={} pool={:?} candidates={} assigned={:?}",
            func.name,
            config.available_regs,
            phase1_intervals.len(),
            ids
        );
    }

    let mut used_regs_set: FxHashSet<u8> = FxHashSet::default();
    for &reg in assignments.values() {
        used_regs_set.insert(reg.0);
    }
    let mut caller_used_regs_set: FxHashSet<u8> = FxHashSet::default();

    // Phase 2: caller-saved for non-spanning leftovers (remats welcome here).
    if !config.caller_saved_regs.is_empty() {
        let i686_pool = config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 4 | 5));
        let hazards: Option<(Vec<u32>, Vec<u32>)> = if i686_pool {
            Some(collect_i686_scratch_hazard_points(
                func,
                &non_gpr_values,
                &FxHashSet::default(),
            ))
        } else {
            None
        };

        let base_ok = |assignments: &FxHashMap<u32, PhysReg>, iv: &LiveInterval| {
            !assignments.contains_key(&iv.value_id)
                && !call_spanning.contains(&iv.value_id)
                && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                && !riscv_entry_guard.contains(&iv.value_id)
        };

        if let Some((ecx_hazards, edx_hazards)) = hazards {
            if env_on("CCC_DEBUG_RA_INTERVALS") {
                eprintln!(
                    "[RA-P2] fn={} ecx_hazards={:?} edx_hazards={:?}",
                    func.name, ecx_hazards, edx_hazards
                );
            }
            for (reg, reg_hazards) in [(PhysReg(5), &edx_hazards), (PhysReg(4), &ecx_hazards)] {
                if !config.caller_saved_regs.contains(&reg) {
                    continue;
                }
                let intervals: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .filter(|iv| !scratch_denied.contains(&iv.value_id))
                    .filter(|iv| !overlaps_inclusive_skip_birth(iv, reg_hazards))
                    .collect();
                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    let cands: Vec<u32> = intervals.iter().map(|iv| iv.value_id).collect();
                    eprintln!(
                        "[RA-P2] fn={} reg={:?} candidates={:?}",
                        func.name, reg, cands
                    );
                }
                if intervals.is_empty() {
                    continue;
                }
                let ranges = build_gpr_ranges(&intervals);
                let mut alloc = LinearScanAllocator::new(ranges, vec![reg]);
                alloc.run();
                for (vid, r) in alloc.assignments {
                    assignments.insert(vid, r);
                    caller_used_regs_set.insert(r.0);
                }
            }
        } else {
            // Split Phase 2 so values consumed as call arguments never take an
            // ABI argument-register home (their home would be clobbered by the
            // staging of an earlier argument). They still get r10/r11-class
            // caller-saved registers; everything else uses the full pool.
            if config.call_arg_regs.is_empty() {
                let phase2_intervals: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .collect();
                if !phase2_intervals.is_empty() {
                    let phase2_ranges = build_gpr_ranges(&phase2_intervals);
                    let mut caller_allocator =
                        LinearScanAllocator::new(phase2_ranges, config.caller_saved_regs.clone());
                    caller_allocator.run();
                    for (vid, reg) in caller_allocator.assignments {
                        assignments.insert(vid, reg);
                        caller_used_regs_set.insert(reg.0);
                    }
                }
            } else {
                let arg_reg_set: FxHashSet<u8> = config.call_arg_regs.iter().map(|r| r.0).collect();
                let indirect_set: FxHashSet<u8> =
                    config.indirect_target_regs.iter().map(|r| r.0).collect();
                let no_arg_pool: Vec<PhysReg> = config
                    .caller_saved_regs
                    .iter()
                    .copied()
                    .filter(|r| !arg_reg_set.contains(&r.0))
                    .collect();
                let no_arg_no_indirect_pool: Vec<PhysReg> = no_arg_pool
                    .iter()
                    .copied()
                    .filter(|r| !indirect_set.contains(&r.0))
                    .collect();
                let no_indirect_pool: Vec<PhysReg> = config
                    .caller_saved_regs
                    .iter()
                    .copied()
                    .filter(|r| !indirect_set.contains(&r.0))
                    .collect();

                // The waves must not reuse each other's registers while a value
                // still lives there: each wave's homes are seeded into the next
                // (a naive split once gave the format string and a div-by-const
                // sign temp the same %r11). Most-constrained wave first so its
                // values are not starved by the later, freer waves.
                let mut seeded: FxHashMap<PhysReg, Vec<(u32, u32)>> = FxHashMap::default();

                // Wave 1: indirect-call args at index ≥ 1 — avoid the arg
                // registers AND the indirect-target register (r10).
                let w1: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .filter(|iv| indirect_arg_values.contains(&iv.value_id))
                    .filter(|iv| later_arg_values.contains(&iv.value_id))
                    .collect();
                if !w1.is_empty() && !no_arg_no_indirect_pool.is_empty() {
                    let ranges = build_gpr_ranges(&w1);
                    let mut alloc = LinearScanAllocator::new(ranges, no_arg_no_indirect_pool);
                    alloc.run();
                    for (vid, reg) in &alloc.assignments {
                        assignments.insert(*vid, *reg);
                        caller_used_regs_set.insert(reg.0);
                        if let Some(&(start, end)) = iv_map.get(vid) {
                            if end > start {
                                // Seed spans are HALF-OPEN in the allocator
                                // (`occupy_register` stores `end + 1`), while
                                // `iv_map` ends are the last LIVE point: push
                                // [start, end+1) or the final live point is
                                // unseeded and a later wave reuses the
                                // register on exactly that point (time_str:
                                // arg0 [16,20] collided with arg2 [19,20] on
                                // %r11 at the call staging point 20).
                                seeded
                                    .entry(*reg)
                                    .or_default()
                                    .push((start, end.saturating_add(1)));
                            }
                        }
                    }
                }

                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    let ids: Vec<(u32, u8)> = w1
                        .iter()
                        .filter(|iv| assignments.contains_key(&iv.value_id))
                        .map(|iv| (iv.value_id, assignments[&iv.value_id].0))
                        .collect();
                    eprintln!("[RA-W1] fn={} homes={:?}", func.name, ids);
                }
                // Wave 2: indirect-call args at index 0 — avoid the indirect-
                // target register only; argument registers are still safe.
                let w2: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .filter(|iv| indirect_arg_values.contains(&iv.value_id))
                    .filter(|iv| !later_arg_values.contains(&iv.value_id))
                    .collect();
                if !w2.is_empty() && !no_indirect_pool.is_empty() {
                    let ranges = build_gpr_ranges(&w2);
                    let mut alloc = LinearScanAllocator::new(ranges, no_indirect_pool);
                    alloc.run_with_seed(&seeded);
                    for (vid, reg) in &alloc.assignments {
                        assignments.insert(*vid, *reg);
                        caller_used_regs_set.insert(reg.0);
                        if let Some(&(start, end)) = iv_map.get(vid) {
                            if end > start {
                                // Half-open seed span (see the Wave 1 note).
                                seeded
                                    .entry(*reg)
                                    .or_default()
                                    .push((start, end.saturating_add(1)));
                            }
                        }
                    }
                }

                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    let ids: Vec<(u32, u8)> = w2
                        .iter()
                        .filter(|iv| assignments.contains_key(&iv.value_id))
                        .map(|iv| (iv.value_id, assignments[&iv.value_id].0))
                        .collect();
                    eprintln!("[RA-W2] fn={} homes={:?}", func.name, ids);
                }
                // Wave 3: direct-call args at index ≥ 1 — avoid the arg
                // registers; the indirect-target register is safe here.
                let w3: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .filter(|iv| later_arg_values.contains(&iv.value_id))
                    .filter(|iv| !indirect_arg_values.contains(&iv.value_id))
                    .collect();
                if !w3.is_empty() && !no_arg_pool.is_empty() {
                    let ranges = build_gpr_ranges(&w3);
                    let mut alloc = LinearScanAllocator::new(ranges, no_arg_pool);
                    alloc.run_with_seed(&seeded);
                    for (vid, reg) in &alloc.assignments {
                        assignments.insert(*vid, *reg);
                        caller_used_regs_set.insert(reg.0);
                        if let Some(&(start, end)) = iv_map.get(vid) {
                            if end > start {
                                // Half-open seed span (see the Wave 1 note).
                                seeded
                                    .entry(*reg)
                                    .or_default()
                                    .push((start, end.saturating_add(1)));
                            }
                        }
                    }
                }

                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    let ids: Vec<(u32, u8)> = w3
                        .iter()
                        .filter(|iv| assignments.contains_key(&iv.value_id))
                        .map(|iv| (iv.value_id, assignments[&iv.value_id].0))
                        .collect();
                    eprintln!("[RA-W3] fn={} homes={:?}", func.name, ids);
                }
                // Wave 4: the rest (non-call-args and direct arg-0 values) —
                // full caller-saved pool.
                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    let mut regs = vec![];
                    for (r, spans) in &seeded {
                        regs.push((r.0, spans.clone()));
                    }
                    regs.sort_unstable();
                    eprintln!("[RA-W4SEED] fn={} seed={:?}", func.name, regs);
                }
                let w4: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| base_ok(&assignments, iv))
                    .filter(|iv| !later_arg_values.contains(&iv.value_id))
                    .filter(|iv| !indirect_arg_values.contains(&iv.value_id))
                    .collect();
                if !w4.is_empty() {
                    let ranges = build_gpr_ranges(&w4);
                    let mut alloc =
                        LinearScanAllocator::new(ranges, config.caller_saved_regs.clone());
                    alloc.run_with_seed(&seeded);
                    for (vid, reg) in alloc.assignments {
                        assignments.insert(vid, reg);
                        caller_used_regs_set.insert(reg.0);
                    }
                    if env_on("CCC_DEBUG_RA_INTERVALS") {
                        let ids: Vec<(u32, u8)> = w4
                            .iter()
                            .filter(|iv| assignments.contains_key(&iv.value_id))
                            .map(|iv| (iv.value_id, assignments[&iv.value_id].0))
                            .collect();
                        eprintln!("[RA-W4] fn={} homes={:?}", func.name, ids);
                    }
                }
            }
        }
    }

    // Phase 2c: leftover callee-saved for call-free overflow.
    {
        let phase2c_intervals: Vec<LiveInterval> = scan_ivs
            .iter()
            .copied()
            .filter(|iv| {
                !assignments.contains_key(&iv.value_id) && !call_spanning.contains(&iv.value_id)
            })
            .collect();
        if !phase2c_intervals.is_empty() {
            let free_callee: Vec<PhysReg> = config
                .available_regs
                .iter()
                .filter(|r| !used_regs_set.contains(&r.0))
                .copied()
                .collect();
            if !free_callee.is_empty() {
                let mut phase2c_ranges = build_gpr_ranges(&phase2c_intervals);
                // A coalesced leader carries only its own uses in the range
                // metadata, so a param merged with a loop-carried copy would
                // look like a single-use value and lose the callee-saved
                // register to the hot loop temps (adler32/memcmp spilled the
                // parameters). Re-weight by the group's total loop-weighted
                // uses before the spill allocator ranks them.
                bump_coalesce_group_priority(&mut phase2c_ranges, &coalesce_groups, &use_count);
                bump_gep_base_priority(&mut phase2c_ranges, &liveness);
                let mut spill_allocator = LinearScanAllocator::new(phase2c_ranges, free_callee);
                spill_allocator.run();
                for (vid, reg) in spill_allocator.assignments {
                    assignments.insert(vid, reg);
                    used_regs_set.insert(reg.0);
                }
            }
        }
    }
    propagate_coalesce_members(&mut assignments, &coalesce_member_of);
    if env_on("CCC_DEBUG_RA_PHASES") {
        let mut ids: Vec<u32> = assignments.keys().copied().collect();
        ids.sort_unstable();
        let spills: Vec<u32> = {
            let mut s: Vec<u32> = scan_ivs
                .iter()
                .map(|iv| iv.value_id)
                .filter(|v| !assignments.contains_key(v))
                .collect();
            s.sort_unstable();
            s
        };
        eprintln!(
            "[RA-FINAL] fn={} assigned={:?} spilled={:?}",
            func.name, ids, spills
        );
    }

    // Phase 2d (i686): load-hazard refinement.  Phase 2 treated every
    // non-alloca Load as a %ecx hazard because a slot-resident pointer must
    // be staged through %ecx to be dereferenced.  Loads whose pointer value
    // actually got a REGISTER home — or never materialise at all (folded
    // absolute globals) — emit direct `movX (%ptr),…` / absolute addressing
    // and never touch %ecx.  Now that assignments exist, recompute the hazard
    // set with the real pointer homes and hand the newly hazard-free
    // caller-saved registers to the values Phase 2 had to refuse.
    if !env_on("CCC_NO_LOAD_HAZARD_REFINE")
        && config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 4 | 5))
    {
        let mut ecx_clean_ptrs: FxHashSet<u32> = FxHashSet::default();
        {
            // Alloca destinations resolve through slot/alignment machinery
            // (OverAligned stages via %ecx) — never direct-dereferenceable.
            let mut alloca_dests: FxHashSet<u32> = FxHashSet::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Alloca { dest, .. } = inst {
                        alloca_dests.insert(dest.0);
                    }
                }
            }
            // A pointer is ecx-clean only if EVERY load using it takes the
            // direct-dereference path: the pointer must be register-resident
            // AND the load's DEST must be register-resident (that is what
            // routes emission through try_emit_load_direct's `movX (%ptr),%d`
            // form).  One slot-dest load with the same pointer would stage
            // through %ecx, so all loads must qualify.
            let mut ptr_all_clean: FxHashMap<u32, bool> = FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Load { ptr, dest, ty, .. } = inst {
                        let gpr32 = matches!(
                            ty,
                            IrType::I8
                                | IrType::U8
                                | IrType::I16
                                | IrType::U16
                                | IrType::I32
                                | IrType::U32
                                | IrType::Ptr
                        );
                        if !gpr32 {
                            continue;
                        }
                        let clean_here = assignments.contains_key(&ptr.0)
                            && !alloca_dests.contains(&ptr.0)
                            && assignments.contains_key(&dest.0);
                        *ptr_all_clean.entry(ptr.0).or_insert(true) &= clean_here;
                    }
                }
            }
            for (ptr, clean) in ptr_all_clean {
                if clean {
                    ecx_clean_ptrs.insert(ptr);
                }
            }
            // Never-materialised pointers (folded absolute globals) emit
            // absolute addressing and touch no scratch register at all.
            for &v in &config.never_materialized {
                ecx_clean_ptrs.insert(v);
            }
        }
        if !ecx_clean_ptrs.is_empty() {
            let (ecx_hazards2, edx_hazards2) = collect_i686_scratch_hazard_points_refined(
                func,
                &non_gpr_values,
                &ecx_clean_ptrs,
                Some(&assignments),
            );
            for (reg, reg_hazards) in [(PhysReg(5), &edx_hazards2), (PhysReg(4), &ecx_hazards2)] {
                if !config.caller_saved_regs.contains(&reg) {
                    continue;
                }
                // Unlike Phases 1/2/2c — which draw from mutually DISJOINT
                // register pools — this refinement re-enters caller-saved
                // registers already handed out by Phase 2.  A candidate must
                // therefore not overlap ANY existing holder of the register,
                // not just its fellow candidates (without this, a loop
                // counter and its bound landed in %edx simultaneously and
                // the bound's leal clobbered the counter).
                let holders: Vec<(u32, u32)> = assignments
                    .iter()
                    .filter(|(_, &r)| r == reg)
                    .filter_map(|(&v, _)| iv_map.get(&v).copied())
                    .collect();
                let intervals: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| {
                        !assignments.contains_key(&iv.value_id)
                            && !call_spanning.contains(&iv.value_id)
                            && !scratch_denied.contains(&iv.value_id)
                            && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                            && !riscv_entry_guard.contains(&iv.value_id)
                    })
                    .filter(|iv| !overlaps_inclusive_skip_birth(iv, reg_hazards))
                    .filter(|iv| {
                        !holders
                            .iter()
                            .any(|&h| intervals_overlap((iv.start, iv.end), h))
                    })
                    .collect();
                if intervals.is_empty() {
                    continue;
                }
                let ranges = build_gpr_ranges(&intervals);
                let mut alloc = LinearScanAllocator::new(ranges, vec![reg]);
                alloc.run();
                for (vid, r) in alloc.assignments {
                    assignments.insert(vid, r);
                    caller_used_regs_set.insert(r.0);
                }
            }
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
        }
    }

    // Phase 2e (i686): %eax as a HOME (lever 3).  The accumulator is a valid
    // register home across straight-line corridors that provably never use
    // %eax as scratch — collect_i686_eax_hazard_points whitelists exactly
    // Phi instructions and Branch/Unreachable terminators, every other
    // emission point is a hazard.  Soundness model:
    //
    //  * DEF POINT: the producer leaves the value in %eax (store_eax_to on an
    //    eax home keeps the cache entry; direct-dest producers write %eax
    //    itself), so a hazard at the def point is birth, not clobber.
    //  * LAST-USE POINT: a hazard there is only safe when the consumer reads
    //    the value from the accumulator BEFORE reusing it — i.e. the value
    //    sits in an accumulator-first operand slot (BinOp/Cmp LHS, Store
    //    val, Cast/UnaryOp/Copy src, Load ptr, Phi incoming, Return
    //    operand).  A binop RHS is read AFTER the LHS is staged through
    //    %eax — homing the RHS in %eax made `xorl %eax,%eax` zero the
    //    accumulator (m32 fuzz seed 0).  `acc_first_uses` records exactly
    //    the values whose EVERY use is such a position.
    //  * any hazard strictly between def and last use destroys the value.
    if !env_on("CCC_NO_EAX_ALLOC")
        && config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 4 | 5))
    {
        let eax_hazards = collect_i686_eax_hazard_points(func);

        // Values whose every operand occurrence is consumed
        // accumulator-first (read from %eax before the consumer reuses it).
        let mut acc_first_uses: FxHashSet<u32> = FxHashSet::default();
        let mut non_acc_first: FxHashSet<u32> = FxHashSet::default();
        let mut mark = |op: &Operand, acc_first: bool| {
            if let Operand::Value(v) = op {
                if acc_first {
                    if !non_acc_first.contains(&v.0) {
                        acc_first_uses.insert(v.0);
                    }
                } else {
                    non_acc_first.insert(v.0);
                    acc_first_uses.remove(&v.0);
                }
            }
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
                        mark(lhs, true);
                        mark(rhs, false);
                    }
                    Instruction::Store { val, ptr, .. } => {
                        mark(val, true);
                        mark(&Operand::Value(*ptr), false);
                    }
                    Instruction::Load { ptr, .. } => {
                        // The pointer is read first (direct dereference or
                        // %ecx staging); the dest cannot share %eax with it
                        // (overlapping intervals), so eax survives to the read.
                        mark(&Operand::Value(*ptr), true);
                    }
                    Instruction::Cast { src, .. } | Instruction::UnaryOp { src, .. } => {
                        mark(src, true);
                    }
                    Instruction::Copy { src, .. } => {
                        mark(src, true);
                    }
                    Instruction::Phi { incoming, .. } => {
                        for (op, _) in incoming {
                            mark(op, true);
                        }
                    }
                    _ => {
                        // GEP/Select/Call args/atomics/intrinsics/inline asm:
                        // operand ordering is not accumulator-first-proven.
                        for_each_operand_in_instruction(inst, |op| mark(op, false));
                    }
                }
            }
            match &block.terminator {
                Terminator::Return(Some(op)) => mark(op, true),
                Terminator::CondBranch { cond, .. } => mark(cond, true),
                _ => for_each_operand_in_terminator(&block.terminator, |op| mark(op, false)),
            }
        }

        let reg = PhysReg(6);
        let holders: Vec<(u32, u32)> = assignments
            .iter()
            .filter(|(_, &r)| r == reg)
            .filter_map(|(&v, _)| iv_map.get(&v).copied())
            .collect();
        let intervals: Vec<LiveInterval> = scan_ivs
            .iter()
            .copied()
            .filter(|iv| {
                !assignments.contains_key(&iv.value_id)
                    && !call_spanning.contains(&iv.value_id)
                    && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                    && !riscv_entry_guard.contains(&iv.value_id)
            })
            .filter(|iv| {
                let idx = eax_hazards.partition_point(|&p| p <= iv.start);
                if idx >= eax_hazards.len() {
                    return true;
                }
                let p = eax_hazards[idx];
                if p > iv.end {
                    return true;
                }
                // A hazard exactly at the last use survives only for
                // accumulator-first consumers (read-then-clobber).
                p == iv.end && acc_first_uses.contains(&iv.value_id)
            })
            .filter(|iv| {
                !holders
                    .iter()
                    .any(|&h| intervals_overlap((iv.start, iv.end), h))
            })
            .collect();
        if !intervals.is_empty() {
            let ranges = build_gpr_ranges(&intervals);
            let mut alloc = LinearScanAllocator::new(ranges, vec![reg]);
            alloc.run();
            for (vid, r) in alloc.assignments {
                assignments.insert(vid, r);
                caller_used_regs_set.insert(r.0);
            }
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
        }
    }

    // AArch64-only: steal a callee-saved from a colder holder for a missed IV.
    // x86 stays out — same eviction already lost gzip inside the scan.
    if !env_on("CCC_NO_LOOP_PIN") && arm_fp_pool && !all_phi_pairs.is_empty() {
        let k: usize = std::env::var("CCC_LOOP_PIN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let phi_pair_values: FxHashSet<u32> = phi_coalesce
            .iter()
            .flat_map(|c| [c.phi_dest, c.backedge_src])
            .collect();
        let mut candidates: Vec<(u32, u64)> = all_phi_pairs
            .iter()
            .map(|c| c.phi_dest)
            .filter(|v| eligible.contains(v))
            .filter(|v| iv_map.get(v).is_some_and(|&(s, e)| e > s))
            .map(|v| (v, use_count.get(&v).copied().unwrap_or(0)))
            .filter(|&(_, count)| count >= 10)
            .collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        candidates.dedup_by_key(|&mut (v, _)| v);

        let overlaps_vid = |a: u32, b: u32| -> bool {
            match (iv_map.get(&a), iv_map.get(&b)) {
                (Some(&ia), Some(&ib)) => intervals_overlap(ia, ib),
                _ => false,
            }
        };
        let mut holders_by_reg: FxHashMap<u8, Vec<u32>> = FxHashMap::default();
        for (&v, &r) in &assignments {
            holders_by_reg.entry(r.0).or_default().push(v);
        }

        let mut steals = 0;
        for &(vid, hot_count) in &candidates {
            if steals >= k || assignments.contains_key(&vid) {
                continue;
            }
            let mut best: Option<(u8, Vec<u32>, u64)> = None;
            for (&reg_id, holders) in &holders_by_reg {
                if !config.available_regs.iter().any(|r| r.0 == reg_id) {
                    continue;
                }
                if holders.iter().any(|h| phi_pair_values.contains(h)) {
                    continue;
                }
                let evict: Vec<u32> = holders
                    .iter()
                    .copied()
                    .filter(|&h| overlaps_vid(h, vid))
                    .collect();
                let cost: u64 = evict
                    .iter()
                    .map(|h| use_count.get(h).copied().unwrap_or(0))
                    .sum();
                if cost >= hot_count {
                    continue;
                }
                if best.as_ref().is_none_or(|&(_, _, c)| cost < c) {
                    best = Some((reg_id, evict, cost));
                }
            }
            if let Some((reg_id, evict, _)) = best {
                for &v in &evict {
                    evict_group(
                        &mut assignments,
                        &mut holders_by_reg,
                        v,
                        &coalesce_member_of,
                        &coalesce_groups,
                    );
                }
                assignments.insert(vid, PhysReg(reg_id));
                holders_by_reg.entry(reg_id).or_default().push(vid);
                steals += 1;
            }
        }
        propagate_coalesce_members(&mut assignments, &coalesce_member_of);
    }
    // Phase 2g (i686): hot-web steal on the narrow 4-register pool.
    //
    // Phase 1's start-point scan order is decisive on a pool this small:
    // long-lived parameters (defined at the entry, spanning every helper
    // call the m16 no-inline policy leaves behind) take ebx..ebp before any
    // loop-carried state is considered, and mode-3 eviction cannot displace
    // them afterwards — a victim whose next use lies anywhere inside the
    // incoming web's span is protected, which is exactly the params' shape.
    // The cmdline parser is the canonical case: option/buf/bufsize parked
    // three callee-saved registers while state/len/bufptr/opptr round-trip
    // stack slots on EVERY iteration — the exact inverse of GCC, which
    // keeps the loop values in registers and reloads the cold parameters
    // on the few paths that read them.
    //
    // Same contract as the AArch64 loop-pin steal above, tightened for the
    // narrow pool:
    //   * candidates are phi-dest loop webs ranked by the WHOLE web's
    //     loop-weighted use count (a state machine's leader alone
    //     undercounts: cmdline's `state` leader shows a fraction of the
    //     web's dispatch reads);
    //   * a register is stolen only when the combined use count of the
    //     holders it would displace is STRICTLY below the candidate's —
    //     global accounting, not the local future-use exchange that lost
    //     gzip when tried inside the scan (mode 5);
    //   * phi-web holders and ABI-hinted holders are never displaced;
    //   * the pass runs once, ranked, before Phase 2f's segment fill (the
    //     segment fill must not see co-holders the steal reasons about).
    let i686_narrow_pool = config
        .caller_saved_regs
        .iter()
        .any(|r| matches!(r.0, 4 | 5))
        && config.available_regs.iter().all(|r| r.0 <= 3);
    if !env_on("CCC_NO_HOT_WEB_STEAL") && i686_narrow_pool && !all_phi_pairs.is_empty() {
        let k: usize = std::env::var("CCC_HOT_WEB_STEAL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        // Web-aware use count: the leader's own count plus every coalesce
        // member's (the state-machine webs the phi-coalesce machinery
        // propagates homes through).
        let web_use_count = |v: u32| -> u64 {
            let mut uc = use_count.get(&v).copied().unwrap_or(0);
            if let Some(members) = coalesce_groups.get(&v) {
                let total: u64 = members
                    .iter()
                    .map(|m| use_count.get(m).copied().unwrap_or(0))
                    .sum();
                uc = uc.max(total);
            }
            uc
        };
        let phi_pair_values: FxHashSet<u32> = phi_coalesce
            .iter()
            .flat_map(|c| [c.phi_dest, c.backedge_src])
            .collect();
        let mut candidates: Vec<(u32, u64)> = all_phi_pairs
            .iter()
            .map(|c| c.phi_dest)
            .filter(|v| eligible.contains(v))
            .filter(|v| iv_map.get(v).is_some_and(|&(s, e)| e > s))
            .map(|v| (v, web_use_count(v)))
            .filter(|&(_, count)| count >= 10)
            .collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        candidates.dedup_by_key(|&mut (v, _)| v);

        let overlaps_vid = |a: u32, b: u32| -> bool {
            match (iv_map.get(&a), iv_map.get(&b)) {
                (Some(&ia), Some(&ib)) => intervals_overlap(ia, ib),
                _ => false,
            }
        };
        let mut holders_by_reg: FxHashMap<u8, Vec<u32>> = FxHashMap::default();
        for (&v, &r) in &assignments {
            holders_by_reg.entry(r.0).or_default().push(v);
        }

        let mut steals = 0;
        for &(vid, hot_count) in &candidates {
            if steals >= k || assignments.contains_key(&vid) {
                continue;
            }
            let mut best: Option<(u8, Vec<u32>, u64)> = None;
            for (&reg_id, holders) in &holders_by_reg {
                if !config.available_regs.iter().any(|r| r.0 == reg_id) {
                    continue;
                }
                if holders
                    .iter()
                    .any(|h| phi_pair_values.contains(h) || config.reg_hints.contains_key(h))
                {
                    continue;
                }
                let evict: Vec<u32> = holders
                    .iter()
                    .copied()
                    .filter(|&h| overlaps_vid(h, vid))
                    .collect();
                if evict.is_empty() {
                    continue;
                }
                let cost: u64 = evict
                    .iter()
                    .map(|&h| use_count.get(&h).copied().unwrap_or(0))
                    .sum();
                if cost >= hot_count {
                    continue;
                }
                if best.as_ref().is_none_or(|&(_, _, c)| cost < c) {
                    best = Some((reg_id, evict, cost));
                }
            }
            if let Some((reg_id, evict, _)) = best {
                for &v in &evict {
                    evict_group(
                        &mut assignments,
                        &mut holders_by_reg,
                        v,
                        &coalesce_member_of,
                        &coalesce_groups,
                    );
                }
                assignments.insert(vid, PhysReg(reg_id));
                holders_by_reg.entry(reg_id).or_default().push(vid);
                steals += 1;
            }
        }
        propagate_coalesce_members(&mut assignments, &coalesce_member_of);
    }

    // Phase 2h (i686): iterated hazard refinement.  Phase 2/2d treated every
    // Div/Rem as an %ecx hazard (divisor staging) and every gpr32 Load as an
    // %edx hazard.  With the final Phase-1..2g assignments in hand, two
    // hazard classes refine (see collect_i686_scratch_hazard_points_refined):
    // div/rem with a register-homed divisor ∉ {edx,eax} is %ecx-clean, and
    // GEP-pointer loads are %edx-clean.  Each round hands the newly-free
    // caller-saved registers to still-unassigned values — which can unlock
    // the next round's refinements (a value homed by round 1 can be the
    // direct divisor that cleans round 2's div points).  Two rounds capture
    // the practical fixpoint; each round is the Phase-2d model: no overlap
    // with refined hazards NOR with existing holders of the register.
    //
    // Boot-corpus payoff: vsprintf number()'s digit loop — the remainder is
    // born in %edx at the fused div, survives the digits GEP (edx-clean) and
    // the indexed load (refined edx-clean), so it keeps %edx and the load
    // folds to `movsbl digits(,%edx),%eax` — GCC's shape, −5 insns/iter.
    if !env_on("CCC_NO_ITERATED_HAZARD")
        && config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 4 | 5))
    {
        // Phase 2d's pointer-cleanliness set (loads whose pointers stage no
        // %ecx) — recomputed here for the refined hazard collection.
        let ecx_clean_ptrs_2h: FxHashSet<u32> = {
            let mut alloca_dests: FxHashSet<u32> = FxHashSet::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Alloca { dest, .. } = inst {
                        alloca_dests.insert(dest.0);
                    }
                }
            }
            let mut ptr_all_clean: FxHashMap<u32, bool> = FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Load { ptr, dest, ty, .. } = inst {
                        let gpr32 = matches!(
                            ty,
                            IrType::I8
                                | IrType::U8
                                | IrType::I16
                                | IrType::U16
                                | IrType::I32
                                | IrType::U32
                                | IrType::Ptr
                        );
                        if !gpr32 {
                            continue;
                        }
                        let clean_here = assignments.contains_key(&ptr.0)
                            && !alloca_dests.contains(&ptr.0)
                            && assignments.contains_key(&dest.0);
                        *ptr_all_clean.entry(ptr.0).or_insert(true) &= clean_here;
                    }
                }
            }
            let mut set: FxHashSet<u32> = ptr_all_clean
                .into_iter()
                .filter(|(_, c)| *c)
                .map(|(v, _)| v)
                .collect();
            for &v in &config.never_materialized {
                set.insert(v);
            }
            set
        };
        for round in 0..2 {
            let (ecx_hazards_h, edx_hazards_h) = collect_i686_scratch_hazard_points_refined(
                func,
                &non_gpr_values,
                &ecx_clean_ptrs_2h,
                Some(&assignments),
            );
            if env_on("CCC_DEBUG_RA_INTERVALS") {
                eprintln!(
                    "[RA-P2h] fn={} round={} refined edx_hazards={:?}",
                    func.name, round, edx_hazards_h
                );
            }
            let mut assigned_this_round = 0usize;
            for (reg, reg_hazards) in [(PhysReg(5), &edx_hazards_h), (PhysReg(4), &ecx_hazards_h)] {
                if !config.caller_saved_regs.contains(&reg) {
                    continue;
                }
                let holders: Vec<(u32, u32)> = assignments
                    .iter()
                    .filter(|(_, &r)| r == reg)
                    .filter_map(|(&v, _)| iv_map.get(&v).copied())
                    .collect();
                if round == 0 && env_on("CCC_DEBUG_RA_INTERVALS") {
                    let holders_dbg: Vec<String> = assignments
                        .iter()
                        .filter(|(_, &r)| r == reg)
                        .filter_map(|(&v, _)| {
                            iv_map
                                .get(&v)
                                .map(|iv| format!("v{}[{}..{}]", v, iv.0, iv.1))
                        })
                        .collect();
                    eprintln!(
                        "[RA-P2h] fn={} reg={:?} holders={:?}",
                        func.name, reg, holders_dbg
                    );
                }
                let intervals: Vec<LiveInterval> = scan_ivs
                    .iter()
                    .copied()
                    .filter(|iv| {
                        !assignments.contains_key(&iv.value_id)
                            && !call_spanning.contains(&iv.value_id)
                            && !scratch_denied.contains(&iv.value_id)
                            && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                            && !riscv_entry_guard.contains(&iv.value_id)
                    })
                    .filter(|iv| !overlaps_inclusive_skip_birth(iv, reg_hazards))
                    .filter(|iv| {
                        !holders
                            .iter()
                            .any(|&h| intervals_overlap((iv.start, iv.end), h))
                    })
                    .collect();
                if env_on("CCC_DEBUG_RA_INTERVALS") {
                    #[allow(unused_mut)]
                    let mut unassigned: Vec<String> = scan_ivs
                        .iter()
                        .filter(|iv| {
                            !assignments.contains_key(&iv.value_id)
                                && !call_spanning.contains(&iv.value_id)
                                && !scratch_denied.contains(&iv.value_id)
                                && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                                && !riscv_entry_guard.contains(&iv.value_id)
                        })
                        .map(|iv| format!("{}:[{}..{}]", iv.value_id, iv.start, iv.end))
                        .collect();
                    let cands: Vec<u32> = intervals.iter().map(|iv| iv.value_id).collect();
                    eprintln!(
                        "[RA-P2h] fn={} round={} reg={:?} unassigned={:?} candidates={:?}",
                        func.name, round, reg, unassigned, cands
                    );
                }
                if intervals.is_empty() {
                    continue;
                }
                let ranges = build_gpr_ranges(&intervals);
                let mut alloc = LinearScanAllocator::new(ranges, vec![reg]);
                alloc.run();
                for (vid, r) in alloc.assignments {
                    assignments.insert(vid, r);
                    caller_used_regs_set.insert(r.0);
                    assigned_this_round += 1;
                }
            }
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
            if assigned_this_round == 0 {
                break;
            }
            if env_on("CCC_DEBUG_RA_INTERVALS") {
                eprintln!(
                    "[RA-P2h] fn={} round={} assigned={}",
                    func.name, round, assigned_this_round
                );
            }
        }
    }

    // Phase 2f (all targets): fill holes in ALREADY-SAVED callee registers
    // using the CFG-aware liveness segments. The primary scan deliberately
    // retains one fat [def,last_use] interval per value, which is simple but
    // makes values on mutually-exclusive diamond/switch arms interfere
    // (the xmltok/inflate 12×/15× stack-memory pathology). Liveness already
    // computes exact conservative segments for call classification; use those
    // segments here as a no-eviction residual coloring step. This is RA-05
    // phase A: `segments` become the primary interference representation for
    // this allocation decision (not just call classification).
    //
    // This phase cannot add a callee-save push/pop: its pool is restricted to
    // registers already present in used_regs_set. It cannot displace an
    // existing home either. A previously slotted value gets a register only
    // when none of its segments intersects current occupancy. Thus the
    // treatment is monotonic in register capacity and fail-closed when a value
    // lacks segment data (its fat interval is used as fallback).
    //
    // Ordering: this runs AFTER the AArch64 loop-pin steal. The steal's
    // holder/evict logic reasons about fat intervals against single-holder
    // registers; a segment-shared co-holder introduced here must not be
    // visible to it (the steal would only evict one of the two holders and
    // could hand the register to a value that fat-overlaps the survivor).
    // i686 validated this fill first (boot C text -1,573 B, stack refs
    // -13.1%); x86-64/AArch64/RISC-V get the same treatment now.
    if !env_on("CCC_NO_SEGMENT_FILL") && !used_regs_set.is_empty() {
        let owner_of = |v: u32| coalesce_member_of.get(&v).copied().unwrap_or(v);

        let mut owned_segments: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
        for seg in &liveness.segments {
            let owner = owner_of(seg.value_id);
            owned_segments
                .entry(owner)
                .or_default()
                .push((seg.start, seg.end));
        }
        // Coalesced members can contribute overlapping/adjacent pieces. Merge
        // them so the interference test remains linear and deterministic.
        for pieces in owned_segments.values_mut() {
            pieces.sort_unstable();
            let source = std::mem::take(pieces);
            insert_segment_union(pieces, &source);
        }
        for iv in &scan_ivs {
            owned_segments
                .entry(iv.value_id)
                .or_insert_with(|| vec![(iv.start, iv.end)]);
        }

        // Collapse current holders to one sorted occupancy set per register.
        // Testing candidates against owner-by-owner vectors is quadratic on
        // sqlite-sized CFGs; the union makes each query independent of the
        // number of SSA holders.
        let mut occupied_by_reg: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        let mut seen_holders: FxHashSet<(u8, u32)> = FxHashSet::default();
        for (&value, &reg) in &assignments {
            if !used_regs_set.contains(&reg.0) {
                continue;
            }
            let owner = owner_of(value);
            if seen_holders.insert((reg.0, owner)) {
                if let Some(segments) = owned_segments.get(&owner) {
                    occupied_by_reg.entry(reg.0).or_default().extend(segments);
                }
            }
        }
        for occupied in occupied_by_reg.values_mut() {
            occupied.sort_unstable();
            let source = std::mem::take(occupied);
            insert_segment_union(occupied, &source);
        }

        let group_pressure = |value: u32| -> u64 {
            let own = use_count.get(&value).copied().unwrap_or(0);
            let group = coalesce_groups.get(&value).map_or(0, |members| {
                members
                    .iter()
                    .map(|m| use_count.get(m).copied().unwrap_or(0))
                    .sum()
            });
            own.max(group)
        };

        // Stack-layout copy aliases may intentionally suppress a Copy's
        // materialization. Assigning such a destination only at this late RA
        // phase creates a register home the alias layer never populates
        // (alias_fuzz_m32 seed 3 caught exactly this). Ordinary producers are
        // authoritative; defer Copy webs to the dedicated coalescers until the
        // location model is unified.
        let copy_dests: FxHashSet<u32> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| match inst {
                Instruction::Copy { dest, .. } => Some(dest.0),
                _ => None,
            })
            .collect();
        let mut candidates: Vec<(u64, u32, u32)> = scan_ivs
            .iter()
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !copy_dests.contains(&iv.value_id))
            .map(|iv| {
                (
                    group_pressure(iv.value_id),
                    iv.end.saturating_sub(iv.start),
                    iv.value_id,
                )
            })
            .collect();
        // Hot/high-use values first; for equal pressure prefer shorter
        // envelopes because they consume fewer holes and admit more followers.
        candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

        let pool: Vec<PhysReg> = config
            .available_regs
            .iter()
            .copied()
            .filter(|r| used_regs_set.contains(&r.0))
            .collect();
        let mut added = 0usize;
        for (_, _, value) in candidates {
            let Some(candidate_segments) = owned_segments.get(&value) else {
                continue;
            };
            let Some(reg) = pool.iter().copied().find(|reg| {
                occupied_by_reg
                    .get(&reg.0)
                    .is_none_or(|occupied| !segment_sets_overlap(candidate_segments, occupied))
            }) else {
                continue;
            };
            if env_on("CCC_DEBUG_SEGMENT_FILL") {
                eprintln!(
                    "[RA-SEGMENT-FILL] fn={} v{} group={:?} segs={:?} -> r{} occupied={:?}",
                    func.name,
                    value,
                    coalesce_groups.get(&value),
                    candidate_segments,
                    reg.0,
                    occupied_by_reg.get(&reg.0)
                );
            }
            assignments.insert(value, reg);
            insert_segment_union(
                occupied_by_reg.entry(reg.0).or_default(),
                candidate_segments,
            );
            added += 1;
        }
        if added != 0 {
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
        }
        if env_on("CCC_DEBUG_SEGMENT_FILL") {
            eprintln!("[RA-SEGMENT-FILL] fn={} added={}", func.name, added);
        }
    }

    if env_on("CCC_DEBUG_RA") {
        let mut v: Vec<(u32, u8)> = assignments.iter().map(|(k, r)| (*k, r.0)).collect();
        v.sort_unstable();
        eprintln!("[RA] fn={} FINAL={:?}", func.name, v);
        if env_on("CCC_DEBUG_RA_INTERVALS") {
            let mut ivs: Vec<(u32, u32, u32, bool, u64)> = iv_map
                .iter()
                .map(|(vid, &(s, e))| {
                    (
                        *vid,
                        s,
                        e,
                        call_spanning.contains(vid),
                        use_count.get(vid).copied().unwrap_or(0),
                    )
                })
                .collect();
            ivs.sort_unstable();
            for (vid, s, e, spans, uses) in ivs {
                let home = match assignments.get(&vid) {
                    Some(r) => format!("reg={}", r.0),
                    None => String::from("SLOT"),
                };
                eprintln!(
                    "[RA-IV] fn={} v{} [{},{}] len={} callspan={} uses={} {}",
                    func.name,
                    vid,
                    s,
                    e,
                    e.saturating_sub(s),
                    spans,
                    uses,
                    home
                );
            }
        }
    }

    apply_phi_coalesce_assignments(
        func,
        &liveness,
        &iv_map,
        &phi_coalesce,
        &mut assignments,
        &config.available_regs,
    );

    let vector_values = if arm_fp_pool {
        collect_vector_values(func)
    } else if x86_fp_pool {
        let mut values = if !env_on("CCC_NO_REDUCTION_VECREG") {
            collect_x86_reduction_vector_values(func)
        } else {
            FxHashSet::default()
        };
        if !env_on("CCC_NO_MAP_VECREG") {
            values.extend(collect_x86_map_broadcast_values(func));
        }
        values
    } else {
        FxHashSet::default()
    };
    let f64_value_set = if arm_fp_pool || (x86_fp_pool && !env_on("CCC_NO_FP_COPY_WEB")) {
        collect_f64_values(func)
    } else {
        FxHashSet::default()
    };

    if !config.xmm_regs.is_empty() {
        let mut real_use: FxHashSet<u32> = FxHashSet::default();
        if arm_fp_pool || x86_fp_pool {
            for block in &func.blocks {
                for inst in &block.instructions {
                    if matches!(inst, Instruction::Copy { .. }) {
                        continue;
                    }
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            real_use.insert(v.0);
                        }
                    });
                }
                for_each_operand_in_terminator(&block.terminator, |op| {
                    if let Operand::Value(v) = op {
                        real_use.insert(v.0);
                    }
                });
            }
            // Fixed-point backward propagation through Copy webs. Must be a
            // fixpoint over ALL copies, not a single-source map: a loop-carried
            // accumulator is defined by one Copy per block (the entry zero AND
            // the backedge FMA result), so a `copy_src_of: dest → src` map keeps
            // only the last edge and strands the entry producer — the pre-loop
            // `VecZero*` lost its YMM home and round-tripped through the stack
            // (p17_dot_f32 / p18_dot_f64 structural regressions).
            loop {
                let mut changed = false;
                for block in &func.blocks {
                    for inst in &block.instructions {
                        if let Instruction::Copy {
                            dest,
                            src: Operand::Value(src_val),
                        } = inst
                        {
                            if real_use.contains(&dest.0) && !real_use.contains(&src_val.0) {
                                real_use.insert(src_val.0);
                                changed = true;
                            }
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }

        let f64_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| non_gpr_values.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !call_spanning.contains(&iv.value_id))
            .filter(|iv| !(arm_fp_pool || x86_fp_pool) || real_use.contains(&iv.value_id))
            .filter(|iv| {
                vector_values.contains(&iv.value_id) || f64_value_set.contains(&iv.value_id)
            })
            .copied()
            .collect();

        if !f64_intervals.is_empty() {
            let mut f64_ranges =
                live_range::build_live_ranges(&f64_intervals, &liveness.block_loop_depth, func);
            // RA-05: hole-aware coverage for the XMM scan as well — FP phi
            // webs spanning mutually exclusive arms get the same treatment
            // as the GPR scan.
            attach_scan_segments(&mut f64_ranges, &liveness, &coalesce_member_of);
            let mut xmm_allocator = LinearScanAllocator::new(f64_ranges, config.xmm_regs.clone());
            xmm_allocator.run();
            for (vid, reg) in xmm_allocator.assignments {
                assignments.insert(vid, reg);
            }
        }

        if !env_on("CCC_NO_VECREG") {
            let vec_candidates = collect_vecreg_candidates(func);
            if !vec_candidates.is_empty() {
                let mut vec_pool: Vec<PhysReg> = (21..=25).map(PhysReg).collect();
                vec_pool.retain(|r| !assignments.values().any(|a| a.0 == r.0));
                if !vec_pool.is_empty() {
                    let vec_intervals =
                        synthetic_vec_intervals(func, &vec_candidates, &liveness.block_loop_depth);
                    let vec_intervals: Vec<LiveInterval> = vec_intervals
                        .into_iter()
                        .filter(|iv| !assignments.contains_key(&iv.value_id))
                        .filter(|iv| !spans_any_call(iv, call_points))
                        .collect();
                    if !vec_intervals.is_empty() {
                        let vec_ranges = live_range::build_live_ranges(
                            &vec_intervals,
                            &liveness.block_loop_depth,
                            func,
                        );
                        let mut vec_allocator = LinearScanAllocator::new(vec_ranges, vec_pool);
                        vec_allocator.run();
                        for (vid, reg) in vec_allocator.assignments {
                            assignments.insert(vid, reg);
                        }
                    }
                }
            }
        }
    }

    if arm_fp_pool {
        for (index, value) in func.loop_promoted_f64_values.iter().take(8).enumerate() {
            assignments.insert(value.0, PhysReg(48 + index as u8));
        }
    }

    if arm_fp_pool || !vector_values.is_empty() || !f64_value_set.is_empty() {
        for candidate in &all_phi_pairs {
            let is_fp = |r: &PhysReg| {
                if arm_fp_pool {
                    (32..=38).contains(&r.0) || (40..=55).contains(&r.0)
                } else {
                    (20..=33).contains(&r.0)
                }
            };
            let d_reg = assignments.get(&candidate.phi_dest).copied().filter(is_fp);
            let s_reg = assignments
                .get(&candidate.backedge_src)
                .copied()
                .filter(is_fp);
            let (Some(d), Some(s)) = (d_reg, s_reg) else {
                continue;
            };
            if d == s {
                continue;
            }
            if !f64_value_set.contains(&candidate.phi_dest)
                && !vector_values.contains(&candidate.phi_dest)
            {
                continue;
            }
            let Some(&src_iv) = iv_map.get(&candidate.backedge_src) else {
                continue;
            };
            let conflict = liveness.intervals.iter().any(|iv| {
                if iv.value_id == candidate.backedge_src || iv.value_id == candidate.phi_dest {
                    return false;
                }
                assignments
                    .get(&iv.value_id)
                    .is_some_and(|&o| o.0 == d.0 && intervals_overlap((iv.start, iv.end), src_iv))
            });
            if !conflict {
                assignments.insert(candidate.backedge_src, d);
            }
        }
    }

    let mut caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
    if !config.caller_saved_regs.is_empty() && env_on("CCC_CALLER_SAVE_SPANNING") {
        let span_regs: Vec<PhysReg> = config
            .caller_saved_regs
            .iter()
            .filter(|r| !caller_used_regs_set.contains(&r.0) && !used_regs_set.contains(&r.0))
            .copied()
            .collect();
        if !span_regs.is_empty() {
            let mut phase2b_intervals: Vec<LiveInterval> = scan_ivs
                .iter()
                .copied()
                .filter(|iv| {
                    !assignments.contains_key(&iv.value_id) && call_spanning.contains(&iv.value_id)
                })
                .collect();
            phase2b_intervals.sort_by(|a, b| {
                use_count
                    .get(&b.value_id)
                    .copied()
                    .unwrap_or(0)
                    .cmp(&use_count.get(&a.value_id).copied().unwrap_or(0))
                    .then_with(|| (a.end - a.start).cmp(&(b.end - b.start)))
            });
            phase2b_intervals.truncate(500);
            if !phase2b_intervals.is_empty() {
                let p2b_map: FxHashMap<u32, (u32, u32)> = phase2b_intervals
                    .iter()
                    .map(|iv| (iv.value_id, (iv.start, iv.end)))
                    .collect();
                let mut phase2b_ranges: Vec<live_range::LiveRange> = phase2b_intervals
                    .iter()
                    .map(|iv| {
                        let mut r = live_range::LiveRange::from_interval(*iv, 0);
                        r.priority = use_count.get(&iv.value_id).copied().unwrap_or(1);
                        r.calculate_spill_weight();
                        r
                    })
                    .collect();
                phase2b_ranges
                    .sort_by(|a, b| a.start.cmp(&b.start).then(b.priority.cmp(&a.priority)));
                let mut span_allocator = LinearScanAllocator::new(phase2b_ranges, span_regs);
                span_allocator.run();
                for (vid, reg) in span_allocator.assignments {
                    assignments.insert(vid, reg);
                    if let Some(&(start, end)) = p2b_map.get(&vid) {
                        caller_save_spans
                            .entry(reg.0)
                            .or_default()
                            .push((start, end.saturating_add(1)));
                    }
                }
                propagate_coalesce_members(&mut assignments, &coalesce_member_of);
            }
        }
    }

    let mut used_regs: Vec<PhysReg> = used_regs_set.iter().map(|&r| PhysReg(r)).collect();
    used_regs.sort_by_key(|r| r.0);

    // ── Post-RA overlap repair (soundness backstop) ────────────────────────
    // The allocator is a federation: linear-scan phases, wave seeds, phi
    // propagation, hot-web/segment fills and steal passes all write
    // `assignments` with their own (sound-in-isolation) occupancy models.
    // The seams leak: a home that is legal for its writer can overlap a
    // home another authority already placed (xxh64_update's pre-homed web
    // held %r10 across [0,223] while later wave/phase values landed on the
    // same register at [6,7) — every preboot-ZSTD pattern failed with
    // "ZSTD-compressed data is corrupt"). CCC_VERIFY_REGALLOC proves the
    // seam after the fact and aborts; this pass REPAIRS the output: of any
    // two coalesce/phi classes whose register's class-union coverage
    // overlaps, the colder class loses its register homes (demoted to
    // stack slots — always sound). One fixpoint-free pass: evictions only
    // remove homes, never add, so no new overlap can appear.
    {
        let mut parent: FxHashMap<u32, u32> = assignments.keys().map(|&v| (v, v)).collect();
        for (&member, &leader) in &coalesce_member_of {
            unite_map(&mut parent, member, leader);
        }
        for pair in &all_phi_pairs {
            unite_map(&mut parent, pair.phi_dest, pair.backedge_src);
        }
        let rep = find_overlapping_classes(&liveness, &assignments, &parent);
        if env_on("CCC_DEBUG_RA_REPAIR") {
            eprintln!(
                "[RA-REPAIR] fn={} scanned assignments={} classes={} overlaps={}",
                func.name,
                assignments.len(),
                parent.len(),
                rep.len()
            );
            for (reg, a, b, s, e) in &rep {
                eprintln!(
                    "[RA-REPAIR] fn={} r{}: class v{} overlaps class v{} at [{},{}] — evicting colder",
                    func.name, reg, a, b, s, e
                );
            }
        }
        if !rep.is_empty() {
            // Evict the colder class of each conflicting pair (fewer total
            // uses; ties break to the LATER value id so the eviction is
            // deterministic).
            let class_weight = |rep_id: u32| -> u64 {
                let mut w = 0u64;
                for (&v, &r) in parent.iter() {
                    if r == rep_id {
                        w += use_count.get(&v).copied().unwrap_or(0);
                    }
                }
                w
            };
            let mut evict_classes: FxHashSet<u32> = FxHashSet::default();
            for &(_reg, a, b, _s, _e) in &rep {
                if evict_classes.contains(&a) || evict_classes.contains(&b) {
                    continue;
                }
                let (wa, wb) = (class_weight(a), class_weight(b));
                let loser = if wa != wb {
                    if wa < wb {
                        a
                    } else {
                        b
                    }
                } else if a < b {
                    b
                } else {
                    a
                };
                evict_classes.insert(loser);
            }
            let mut evicted: Vec<u32> = Vec::new();
            for (&v, &r) in parent.iter() {
                if evict_classes.contains(&r) && assignments.remove(&v).is_some() {
                    evicted.push(v);
                }
            }
            if env_on("CCC_DEBUG_RA_REPAIR") {
                eprintln!(
                    "[RA-REPAIR] fn={} evicted {:?} (classes {:?})",
                    func.name, evicted, evict_classes
                );
            }
        }
    }

    if env_on("CCC_VERIFY_REGALLOC") {
        verify_no_overlap(&liveness, &assignments, &coalesce_member_of, &all_phi_pairs);
    }

    // Session-28 debug: per-value home census (register vs slot) for one
    // function, to analyze spill/slot-traffic decisions.
    if std::env::var_os("LCCC_DBG_RA").is_some() {
        let filter = std::env::var("LCCC_DBG_RA_FUNC").unwrap_or_default();
        if filter.is_empty() || func.name.contains(&filter) {
            let mut rows: Vec<(u32, u32, u32, String)> = Vec::new();
            for iv in &liveness.intervals {
                let home = match assignments.get(&iv.value_id) {
                    Some(r) => format!("r{}", r.0),
                    None => "slot".to_string(),
                };
                let uc = use_count.get(&iv.value_id).copied().unwrap_or(0);
                rows.push((
                    iv.start,
                    iv.end,
                    iv.value_id,
                    format!(
                        "{} uses={} elig={}",
                        home,
                        uc,
                        eligible.contains(&iv.value_id)
                    ),
                ));
            }
            rows.sort();
            eprintln!(
                "[RA] fn={} values={} assigned={}",
                func.name,
                rows.len(),
                assignments.len()
            );
            for (s, e, vid, info) in rows {
                eprintln!("[RA]   v{:>5} [{:>5}, {:>5}] {}", vid, s, e, info);
            }
        }
    }

    if let Ok(filter) = std::env::var("CCC_TRACE_ALLOCSTATS") {
        if filter.is_empty() || filter == "*" || func.name.contains(&filter) {
            let scan_values: FxHashSet<u32> = scan_ivs.iter().map(|iv| iv.value_id).collect();
            let assigned_scan = scan_values
                .iter()
                .filter(|v| assignments.contains_key(v))
                .count();
            let spilled = scan_values.len().saturating_sub(assigned_scan);
            let segment_values: FxHashSet<u32> =
                liveness.segments.iter().map(|s| s.value_id).collect();
            let holes = liveness.segments.len().saturating_sub(segment_values.len());
            let caller_ids: FxHashSet<u8> = config.caller_saved_regs.iter().map(|r| r.0).collect();
            let callee_ids: FxHashSet<u8> = config.available_regs.iter().map(|r| r.0).collect();
            let caller_homes = assignments
                .values()
                .filter(|r| caller_ids.contains(&r.0))
                .count();
            let callee_homes = assignments
                .values()
                .filter(|r| callee_ids.contains(&r.0))
                .count();
            eprintln!(
                "[RA-STATS] fn={} eligible={} scan={} assigned={} spilled={} segments={} holes={} callee-homes={} caller-homes={}",
                func.name,
                eligible.len(),
                scan_values.len(),
                assigned_scan,
                spilled,
                liveness.segments.len(),
                holes,
                callee_homes,
                caller_homes
            );
        }
    }

    if let Ok(filter) = std::env::var("CCC_RA_EXPLAIN") {
        if filter.is_empty() || filter == "*" || func.name.contains(&filter) {
            let mut segment_count: FxHashMap<u32, usize> = FxHashMap::default();
            for segment in &liveness.segments {
                *segment_count.entry(segment.value_id).or_insert(0) += 1;
            }
            let mut spills: Vec<LiveInterval> = scan_ivs
                .iter()
                .copied()
                .filter(|iv| !assignments.contains_key(&iv.value_id))
                .collect();
            spills.sort_unstable_by_key(|iv| (iv.start, iv.value_id));
            eprintln!(
                "[RA-EXPLAIN] fn={} spills={} assigned={} fat-values={}",
                func.name,
                spills.len(),
                assignments.len(),
                scan_ivs.len()
            );
            for iv in spills {
                let reason = if call_spanning.contains(&iv.value_id) {
                    "callee-pressure"
                } else if param_restricted.contains(&iv.value_id) {
                    "parameter-restricted"
                } else {
                    "hazard-or-register-pressure"
                };
                eprintln!(
                    "[RA-EXPLAIN] spill v{} range=[{},{}] segments={} uses={} reason={}",
                    iv.value_id,
                    iv.start,
                    iv.end,
                    segment_count.get(&iv.value_id).copied().unwrap_or(0),
                    use_count.get(&iv.value_id).copied().unwrap_or(0),
                    reason
                );
            }
        }
    }

    let mut accumulator_assignments =
        analyze_accumulator_assignments(func, config.accumulator_policy);
    // A physical assignment is the durable home and always wins. Publishing
    // both locations made downstream behavior depend on insertion order.
    accumulator_assignments.retain(|a| !assignments.contains_key(&a.value_id));
    verify_accumulator_assignments(func, &accumulator_assignments);

    RegAllocResult {
        assignments,
        accumulator_assignments,
        used_regs,
        caller_save_spans,
        liveness: Some(liveness),
    }
}

/// Verify the final assignment against hole-aware liveness in O(n log n).
///
/// Copy/coalesce and proven phi-destructive-update classes intentionally share
/// one register even where their raw SSA ranges touch; collapse each such
/// equivalence class before checking. Every other overlap is a hard allocator
/// bug. This runs only under `CCC_VERIFY_REGALLOC`, so a failure must abort
/// rather than print a warning that automated validation can miss.
fn verify_no_overlap(
    liveness: &LivenessResult,
    assignments: &FxHashMap<u32, PhysReg>,
    coalesce_member_of: &FxHashMap<u32, u32>,
    phi_pairs: &[PhiCoalesceCandidate],
) {
    fn find(parent: &mut FxHashMap<u32, u32>, value: u32) -> u32 {
        let mut root = value;
        while parent.get(&root).copied().is_some_and(|p| p != root) {
            root = parent[&root];
        }
        let mut current = value;
        while parent.get(&current).copied().is_some_and(|p| p != root) {
            let next = parent[&current];
            parent.insert(current, root);
            current = next;
        }
        root
    }
    fn unite(parent: &mut FxHashMap<u32, u32>, a: u32, b: u32) {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent.insert(rb, ra);
        }
    }

    let mut parent: FxHashMap<u32, u32> = assignments.keys().map(|&v| (v, v)).collect();
    for (&member, &leader) in coalesce_member_of {
        unite(&mut parent, member, leader);
    }
    for pair in phi_pairs {
        unite(&mut parent, pair.phi_dest, pair.backedge_src);
    }

    let mut group_segments: FxHashMap<(u8, u32), Vec<(u32, u32)>> = FxHashMap::default();
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for segment in &liveness.segments {
        let Some(&reg) = assignments.get(&segment.value_id) else {
            continue;
        };
        segmented.insert(segment.value_id);
        let rep = find(&mut parent, segment.value_id);
        group_segments
            .entry((reg.0, rep))
            .or_default()
            .push((segment.start, segment.end));
    }
    // Synthetic values can be absent from `segments`; preserve the verifier's
    // fail-closed behavior with their fat interval.
    for interval in &liveness.intervals {
        if segmented.contains(&interval.value_id) {
            continue;
        }
        let Some(&reg) = assignments.get(&interval.value_id) else {
            continue;
        };
        let rep = find(&mut parent, interval.value_id);
        group_segments
            .entry((reg.0, rep))
            .or_default()
            .push((interval.start, interval.end));
    }

    let mut by_reg: FxHashMap<u8, Vec<(u32, u32, u32)>> = FxHashMap::default();
    for ((reg, rep), pieces) in &mut group_segments {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_segment_union(pieces, &source);
        by_reg
            .entry(*reg)
            .or_default()
            .extend(pieces.iter().map(|&(start, end)| (start, end, *rep)));
    }

    for (reg, events) in &mut by_reg {
        events.sort_unstable();
        let mut previous: Option<(u32, u32, u32)> = None;
        for &(start, end, rep) in events.iter() {
            if let Some((prev_start, prev_end, prev_rep)) = previous {
                assert!(
                    start >= prev_end,
                    "register-allocation overlap: r{} class v{}[{}, {}) vs class v{}[{}, {})",
                    reg,
                    prev_rep,
                    prev_start,
                    prev_end,
                    rep,
                    start,
                    end
                );
                if end <= prev_end {
                    continue;
                }
            }
            previous = Some((start, end, rep));
        }
    }
}

/// 128-bit VECTOR VALUES safe to hold in an XMM for their whole live range.
///
/// A vector value here is the ALLOCA a SIMD intrinsic writes its 16-byte
/// result into (`dest_ptr` of e.g. Pcmpeqb128 / Pxor128). The backend
/// rewrites `movdqu %xmm0, slot` into `movdqa %xmm0, %xmmN` and every
/// subsequent cache-aware vector load into `movdqa %xmmN, %xmm0`.
///
/// Guards (each mirrors a codegen constraint — a candidate outside this
/// whitelist miscompiles, e.g. the fold_4 class):
/// 1. V is an Alloca and the dest_ptr of at least one 128-bit *compute*
///    producer (NOT a store-target op, NOT a mem-load whose dest_ptr is
///    still a slot but whose args are addresses).
/// 2. V is not volatile and not over-aligned beyond 16.
/// 3. V has >= 2 intrinsic-ARG uses, or is a dest_ptr+arg RMW (loop acc).
///    Single-use non-RMW is already handled by deferred-store.
/// 4. Every use is dest_ptr of a 128-producer or an ARG of a *compute*
///    128-producer (sse_load_arg / avx_load_arg_to). Store-target args,
///    mem-load args, raw FMA/horiz/auto-vec readers, and any unknown
///    intrinsic are fail-closed (bad). A new intrinsic must be added to
///    the compute whitelist or it silently refuses — never miscompiles.
/// 5. Never used by a non-Intrinsic instruction or a terminator.
/// 6. Non-call-spanning — enforced by the caller via `spans_any_call`.
fn collect_vecreg_candidates(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp;

    let mut allocas: FxHashSet<u32> = FxHashSet::default();
    let mut volatile_allocas: FxHashSet<u32> = FxHashSet::default();
    let mut over_align_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                align,
                ..
            } = inst
            {
                allocas.insert(dest.0);
                if *volatile || *semantic_volatile {
                    volatile_allocas.insert(dest.0);
                }
                if *align > 16 {
                    over_align_allocas.insert(dest.0);
                }
            }
        }
    }

    // Compute producers: dest_ptr is a 16-byte slot and the first N args
    // are vector slots loaded via sse_load_arg.  Return the exact VECTOR
    // prefix length rather than an op-level bool: several instructions carry
    // trailing scalar/immediate operands.  Treating every Value operand as a
    // vector pointer can admit an alloca used as an immediate/scalar and make
    // codegen reinterpret its XMM contents as an address.
    //
    // Keep this fail-closed and tied to lowering shapes.  New intrinsics get
    // no vecreg allocation until their sse_load_arg/sse_store_dest contract is
    // audited here.
    let compute_vec_arg_count = |op: &IntrinsicOp| -> Option<usize> {
        use IntrinsicOp as O;
        match op {
            // Full-width constants do not read a vector operand.
            O::Setzero128 => Some(0),

            // Unary vector input followed, for some ops, by scalar/immediate
            // operands. Inserts preserve the other lanes of args[0].
            O::Pabsb128
            | O::Pabsw128
            | O::Pabsd128
            | O::Pmovzxbw128
            | O::Pmovzxwd128
            | O::Aesimc128
            | O::Aeskeygenassist128
            | O::Psllwi128
            | O::Psrlwi128
            | O::Psrawi128
            | O::Psradi128
            | O::Pslldi128
            | O::Psrldi128
            | O::Pslldqi128
            | O::Psrldqi128
            | O::Psllqi128
            | O::Psrlqi128
            | O::Pshufd128
            | O::Pshuflw128
            | O::Pshufhw128
            | O::Pinsrw128
            | O::Pinsrd128
            | O::Pinsrb128
            | O::Pinsrq128 => Some(1),

            // Three genuine vector inputs (no scalar operand in the prefix).
            O::Pblendvb128
            | O::Dpbusd128
            | O::Dpbusds128
            | O::Dpwusd128
            | O::Dpwusds128
            | O::Dpbssd128
            | O::Dpbssds128
            | O::Dpbsud128
            | O::Dpbsuds128
            | O::Dpbuud128
            | O::Dpbuuds128
            | O::Dpwuud128
            | O::Dpwuuds128
            | O::Dpwssd128
            | O::Dpwssds128 => Some(3),

            // Binary vector inputs.  Palignr/Pblendw/Pclmul/GFNI append an
            // immediate after this two-vector prefix; variable shifts really
            // do consume their count operand as a 128-bit vector.
            O::Pcmpeqb128
            | O::Pcmpeqd128
            | O::Psubusb128
            | O::Psubsb128
            | O::Por128
            | O::Pand128
            | O::Pxor128
            | O::AddPs128
            | O::SubPs128
            | O::MulPs128
            | O::AddPd128
            | O::SubPd128
            | O::MulPd128
            | O::Paddw128
            | O::Psubw128
            | O::Pmulhw128
            | O::Pmullw128
            | O::Pmuludq128
            | O::Pmuldq128
            | O::Pmulld128
            | O::Pmaddwd128
            | O::Pmaddubsw128
            | O::Pcmpgtw128
            | O::Pcmpgtb128
            | O::Paddd128
            | O::Psubd128
            | O::Paddb128
            | O::Psubb128
            | O::Psubusw128
            | O::Psadbw128
            | O::Pshufb128
            | O::Pmaxub128
            | O::Pminub128
            | O::Packssdw128
            | O::Packsswb128
            | O::Packuswb128
            | O::Punpcklbw128
            | O::Punpckhbw128
            | O::Punpcklwd128
            | O::Punpckhwd128
            | O::Phaddw128
            | O::Phaddd128
            | O::Palignr128
            | O::Psllw128
            | O::Psrlw128
            | O::Pblendw128
            | O::Aesenc128
            | O::Aesenclast128
            | O::Aesdec128
            | O::Aesdeclast128
            | O::Pclmulqdq128
            | O::Gf2p8mulb128
            | O::Gf2p8affineqb128
            | O::Gf2p8affineinvqb128
            | O::AddF64x2
            | O::MulF64x2
            | O::AddI32x4
            // SSE2 operations wired after the original whitelist.  Their
            // lowerings all route through emit_sse_binary_128.
            | O::Paddusb128
            | O::Paddsb128
            | O::Paddusw128
            | O::Paddsw128
            | O::Psubsw128
            | O::Pandn128
            | O::Pcmpeqw128
            | O::Pcmpgtd128
            | O::Pavgb128
            | O::Pavgw128
            | O::Pminsw128
            | O::Pmaxsw128
            | O::Pmulhuw128
            | O::Paddq128
            | O::Psubq128
            | O::Punpckldq128
            | O::Punpckhdq128
            | O::Punpcklqdq128
            | O::Punpckhqdq128 => Some(2),
            _ => None,
        }
    };
    let is_128_mem_load = |op: &IntrinsicOp| -> bool {
        use IntrinsicOp as O;
        matches!(
            op,
            O::Loaddqu
                | O::Loadldi128
                | O::SetEpi8
                | O::SetEpi16
                | O::SetEpi32
                | O::Cvtsi32Si128
                | O::Cast256to128
        )
    };
    let is_store_target = |op: &IntrinsicOp| -> bool {
        matches!(
            op,
            IntrinsicOp::Storedqu
                | IntrinsicOp::Storeu256
                | IntrinsicOp::Store256
                | IntrinsicOp::Storeldi128
                | IntrinsicOp::Movntdq
                | IntrinsicOp::Movntpd
        )
    };
    let is_raw_reader = |op: &IntrinsicOp| -> bool {
        use IntrinsicOp as O;
        matches!(
            op,
            O::FmaF64x2
                | O::FmaF64x4
                | O::FmaF64x4Hoisted
                | O::FmaF64x4SIB
                | O::FmaF64x4HoistedSIB
                | O::BroadcastLoadF64
                | O::LoadF64x2
                | O::LoadF64x4
                | O::LoadI32x4
                | O::LoadI32x8
                | O::HorizontalAddF64x2
                | O::HorizontalAddF64x4
                | O::HorizontalAddI32x4
                | O::HorizontalAddI32x8
                | O::VecLoadF64x2
                | O::VecLoadF64x4
                | O::VecLoadI32x4
                | O::VecLoadI32x8
                | O::VecLoadF32x4
                | O::VecLoadF32x8
                | O::VecAddF64x2
                | O::VecAddF64x4
                | O::VecAddI32x4
                | O::VecAddI32x8
                | O::VecAddF32x4
                | O::VecAddF32x8
                | O::VecMulF64x2
                | O::VecMulF64x4
                | O::VecMulF32x4
                | O::VecMulF32x8
                | O::VecFmaF64x4
                | O::VecFmaF32x8
                | O::VecHorizontalAddF64x2
                | O::VecHorizontalAddF64x4
                | O::VecHorizontalAddI32x4
                | O::VecHorizontalAddI32x8
                | O::VecHorizontalAddF32x4
                | O::VecHorizontalAddF32x8
                | O::VecZeroF32x4
                | O::VecZeroF32x8
        )
    };

    let mut produced: FxHashSet<u32> = FxHashSet::default();
    let mut store_target: FxHashSet<u32> = FxHashSet::default();
    let mut arg_uses: FxHashMap<u32, u32> = FxHashMap::default();
    let mut rmw: FxHashSet<u32> = FxHashSet::default();
    let mut bad_use: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Intrinsic {
                    dest_ptr: Some(d),
                    op,
                    args,
                    ..
                } => {
                    if compute_vec_arg_count(op).is_some() || is_128_mem_load(op) {
                        produced.insert(d.0);
                    }
                    if is_store_target(op) {
                        store_target.insert(d.0);
                    }
                    // Fail-closed: only compute-producer ARGs are cache-aware
                    // vector reads. Everything else (store payload, mem-load
                    // address, raw FMA/horiz, unknown op) poisons the value.
                    let vec_arg_count = compute_vec_arg_count(op).unwrap_or(0);
                    for (index, arg) in args.iter().enumerate() {
                        if let Operand::Value(v) = arg {
                            if index < vec_arg_count {
                                *arg_uses.entry(v.0).or_insert(0) += 1;
                                if v.0 == d.0 {
                                    rmw.insert(v.0);
                                }
                            } else {
                                bad_use.insert(v.0);
                            }
                        }
                    }
                    if is_raw_reader(op) {
                        for arg in args {
                            if let Operand::Value(v) = arg {
                                bad_use.insert(v.0);
                            }
                        }
                    }
                }
                Instruction::Intrinsic {
                    dest_ptr: None,
                    op,
                    args,
                    ..
                } => {
                    // No dest_ptr: still poison args unless this is a known
                    // compute op (should not happen for the 128 family).
                    let vec_arg_count = compute_vec_arg_count(op).unwrap_or(0);
                    for (index, arg) in args.iter().enumerate() {
                        if let Operand::Value(v) = arg {
                            if index < vec_arg_count {
                                *arg_uses.entry(v.0).or_insert(0) += 1;
                            } else {
                                bad_use.insert(v.0);
                            }
                        }
                    }
                }
                other => {
                    for_each_operand_in_instruction(other, |op| {
                        if let Operand::Value(v) = op {
                            bad_use.insert(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(other, |v| {
                        bad_use.insert(v.0);
                    });
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                bad_use.insert(v.0);
            }
        });
    }

    let mut result = FxHashSet::default();
    for &v in &produced {
        if !allocas.contains(&v)
            || store_target.contains(&v)
            || volatile_allocas.contains(&v)
            || over_align_allocas.contains(&v)
            || bad_use.contains(&v)
        {
            continue;
        }
        let uses = arg_uses.get(&v).copied().unwrap_or(0);
        if uses < 2 && !rmw.contains(&v) {
            continue;
        }
        result.insert(v);
    }
    result
}

/// Synthetic live intervals for vecreg allocas (liveness excludes them).
/// Same program-point convention as `liveness::assign_program_points`:
/// +1 per instruction, +1 per terminator.
///
/// Half-open `[first_mention, last_mention + 1)`. A same-instruction RMW
/// (`acc` is both dest_ptr and arg) is `[p, p+1)` so the scan accepts it
/// — the old `e > s` closed interval dropped every single-site loop acc.
///
/// Linear points do not wrap around back-edges. A value mentioned in a
/// loop is therefore grown to the entire contiguous depth>0 *region* it
/// touches (layout-adjacent loop blocks). Sequential separate loops stay
/// disjoint; a split-layout single loop (non-contiguous depth>0 runs
/// that both mention V) spans from the first touched region to the last.
fn synthetic_vec_intervals(
    func: &IrFunction,
    candidates: &FxHashSet<u32>,
    block_loop_depth: &[u32],
) -> Vec<LiveInterval> {
    let n = func.blocks.len();
    let mut block_range: Vec<(u32, u32)> = Vec::with_capacity(n);
    let mut first_at: FxHashMap<u32, u32> = FxHashMap::default();
    let mut last_at: FxHashMap<u32, u32> = FxHashMap::default();
    let mut mentioned_in: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    let mut point: u32 = 0;

    for (bi, block) in func.blocks.iter().enumerate() {
        let bstart = point;
        for inst in &block.instructions {
            let mut hit = false;
            if let Instruction::Intrinsic {
                dest_ptr: Some(d), ..
            } = inst
            {
                if candidates.contains(&d.0) {
                    first_at.entry(d.0).or_insert(point);
                    last_at.insert(d.0, point);
                    hit = true;
                    mentioned_in.entry(d.0).or_default().push(bi);
                }
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if candidates.contains(&v.0) {
                        first_at.entry(v.0).or_insert(point);
                        last_at.insert(v.0, point);
                        if !hit {
                            mentioned_in.entry(v.0).or_default().push(bi);
                        }
                    }
                }
            });
            point += 1;
        }
        point += 1; // terminator
        block_range.push((bstart, point));
    }

    // Contiguous depth>0 runs in layout order.
    let mut region_of: Vec<Option<usize>> = vec![None; n];
    let mut regions: Vec<(u32, u32)> = Vec::new();
    let mut i = 0;
    while i < n {
        if block_loop_depth.get(i).copied().unwrap_or(0) == 0 {
            i += 1;
            continue;
        }
        let rid = regions.len();
        let rstart = block_range[i].0;
        let mut j = i;
        while j < n && block_loop_depth.get(j).copied().unwrap_or(0) > 0 {
            region_of[j] = Some(rid);
            j += 1;
        }
        regions.push((rstart, block_range[j - 1].1));
        i = j;
    }

    let mut result = Vec::new();
    for &v in candidates {
        let Some(&s0) = first_at.get(&v) else {
            continue;
        };
        let e0 = last_at.get(&v).copied().unwrap_or(s0).saturating_add(1);
        let (mut s, mut e) = (s0.min(e0.saturating_sub(1)), e0.max(s0.saturating_add(1)));
        if e <= s {
            e = s.saturating_add(1);
        }

        if let Some(blocks) = mentioned_in.get(&v) {
            let mut touched: Vec<usize> = blocks
                .iter()
                .filter_map(|&bi| region_of.get(bi).copied().flatten())
                .collect();
            if !touched.is_empty() {
                touched.sort_unstable();
                touched.dedup();
                if touched.len() == 1 {
                    let (rs, re) = regions[touched[0]];
                    s = s.min(rs);
                    e = e.max(re);
                } else {
                    // Split layout of one loop, or V used in two loops:
                    // span the envelope. Over-approx, never a wrong assign.
                    let rs = regions[touched[0]].0;
                    let re = regions[*touched.last().unwrap()].1;
                    s = s.min(rs);
                    e = e.max(re);
                }
            }
        }

        if e > s {
            result.push(LiveInterval {
                value_id: v,
                start: s,
                end: e,
            });
        }
    }
    result
}

/// `src → dest` Copy edges. Shared by the three copy-web collectors so
/// sqlite-sized functions do O(n) BFS instead of O(n · chain) fixpoints.
fn copy_successors(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    let mut succs: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(s),
            } = inst
            {
                if s.0 != dest.0 {
                    succs.entry(s.0).or_default().push(dest.0);
                }
            }
        }
    }
    succs
}

fn propagate_copy_web(succs: &FxHashMap<u32, Vec<u32>>, seed: &mut FxHashSet<u32>) {
    let mut work: Vec<u32> = seed.iter().copied().collect();
    while let Some(id) = work.pop() {
        if let Some(ds) = succs.get(&id) {
            for &d in ds {
                if seed.insert(d) {
                    work.push(d);
                }
            }
        }
    }
}

/// Values whose (last) use is as an operand of a `Call` / `CallIndirect`.
///
/// The call's argument staging materialises each argument into the ABI arg
/// registers in order; a value homed in one of those registers (x86-64:
/// rdi/rsi/rdx/r8/r9) is read only AFTER an earlier argument's staging already
/// wrote that register. Phase 2 therefore allocates these values from the
/// arg-register-free subset of the caller-saved pool. The second set is the
/// subset used as arguments to a `CallIndirect`, whose staging also writes the
/// indirect-target register (r10) before reading them.
/// Values whose (last) use is as an operand of a `Call` / `CallIndirect`.
///
/// Returns `(later, indirect)`:
/// * `later` — values used as a call argument at ABI index ≥ 1 (i.e. NOT the
///   first argument). Their home is read only after the staging has written
///   the argument register(s) that come before them in the ABI order, so those
///   registers must not be their home.
/// * `indirect` — values used as an argument of a `CallIndirect`, whose
///   staging additionally writes the indirect-target register (x86: `%r10`)
///   before reading any argument.
///
/// A value used as argument 0 of a DIRECT call is deliberately absent from
/// both: argument 0 is read first, so every argument register (and r10/r11)
/// is still safe as its home.
fn collect_call_arg_values(func: &IrFunction) -> (FxHashSet<u32>, FxHashSet<u32>) {
    let mut later = FxHashSet::default();
    let mut indirect = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let (args, is_indirect) = match inst {
                Instruction::Call { info, .. } => (&info.args, false),
                Instruction::CallIndirect { info, .. } => (&info.args, true),
                _ => continue,
            };
            for (idx, arg) in args.iter().enumerate() {
                if let Operand::Value(v) = arg {
                    if idx >= 1 {
                        later.insert(v.0);
                    }
                    if is_indirect {
                        indirect.insert(v.0);
                    }
                }
            }
        }
    }
    (later, indirect)
}

/// Values that do not fit in a single GPR (floats, i128, 32-bit i64/u64),
/// plus Copy destinations chained from them.
///
/// Signature matches Part 1: `collect_non_gpr_values(func, is_32bit)`.
/// `produces_vector_value()` is the single source of truth for the vector
/// set (x86 `Vec*` and the AArch64 I64x2 widening family).
fn collect_non_gpr_values(func: &IrFunction, is_32bit: bool) -> FxHashSet<u32> {
    let mut non_gpr_values: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. } | Instruction::UnaryOp { dest, ty, .. } => {
                    if is_non_gpr_type(ty, is_32bit) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Cast {
                    dest,
                    to_ty,
                    from_ty,
                    ..
                } => {
                    if is_non_gpr_type(to_ty, is_32bit) || is_non_gpr_type(from_ty, is_32bit) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Load { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. } => {
                    if is_non_gpr_type(ty, is_32bit) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        if is_non_gpr_type(&info.return_type, is_32bit) {
                            non_gpr_values.insert(dest.0);
                        }
                    }
                }
                Instruction::Copy { dest, src } => {
                    let src_is_non_gpr = match src {
                        Operand::Const(IrConst::F32(_))
                        | Operand::Const(IrConst::F64(_))
                        | Operand::Const(IrConst::LongDouble(..))
                        | Operand::Const(IrConst::I128(_)) => true,
                        Operand::Const(IrConst::I64(_)) if is_32bit => true,
                        _ => false,
                    };
                    if src_is_non_gpr {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Intrinsic {
                    dest: Some(d), op, ..
                } => {
                    use crate::ir::intrinsics::IntrinsicOp;
                    if matches!(
                        op,
                        IntrinsicOp::SqrtF64
                            | IntrinsicOp::SqrtF32
                            | IntrinsicOp::FabsF64
                            | IntrinsicOp::FabsF32
                            | IntrinsicOp::FmaScalarF64
                            | IntrinsicOp::FmaScalarF32
                            | IntrinsicOp::RoundScalarF64(_)
                            | IntrinsicOp::RoundScalarF32(_)
                            | IntrinsicOp::CopysignF64
                            | IntrinsicOp::CopysignF32
                            | IntrinsicOp::FixedDistanceF32x8
                            | IntrinsicOp::FixedDistanceF64x4
                    ) || op.produces_vector_value()
                    {
                        non_gpr_values.insert(d.0);
                    }
                }
                _ => {}
            }
        }
    }

    let succs = copy_successors(func);
    propagate_copy_web(&succs, &mut non_gpr_values);
    non_gpr_values
}

/// x86 auto-vectorizer reduction values whose complete def/use web is
/// understood by the width-aware XMM/YMM emitter. Deliberately much
/// narrower than `produces_vector_value()`: arbitrary user-intrinsic values
/// can flow to memcpy, stores, or width-changing ops which still need
/// protected stack homes.
///
/// Accepted web: F32/F64 zero/load/add/mul/fma producers, same-width
/// Copies, and a final same-width horizontal reduction. Keeps 128- and
/// 256-bit values disjoint while admitting loop-carried accumulators.
///
/// Calls are NOT a function-wide ban — Part 1's `spans_any_call` already
/// refuses any interval that is live across a call. InlineAsm / Memcpy
/// clobber XMM without a call_point and still poison the whole function.
fn collect_x86_reduction_vector_values(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp as O;

    let class_of = |op: &O| -> Option<u8> {
        match op {
            O::VecZeroF32x8
            | O::VecLoadF32x8
            | O::VecAddF32x8
            | O::VecMulF32x8
            | O::VecFmaF32x8 => Some(1),
            O::VecZeroF64x4
            | O::VecLoadF64x4
            | O::VecAddF64x4
            | O::VecMulF64x4
            | O::VecFmaF64x4 => Some(2),
            O::VecZeroF32x4 | O::VecLoadF32x4 | O::VecAddF32x4 | O::VecMulF32x4 => Some(3),
            O::VecZeroF64x2 | O::VecLoadF64x2 | O::VecAddF64x2 | O::VecMulF64x2 => Some(4),
            O::VecZeroI32x8 | O::VecLoadI32x8 | O::VecAddI32x8 | O::VecMulI32x8 => Some(5),
            // v12 Fix F: Max reduction producers. VecBroadcastI32x8 seeds the
            // max accumulator (init = arr[0] broadcast), VecMaxI32x8 produces
            // the new accumulator each iteration. Classifying them as class 5
            // (I32x8) lets the Copy-web connect the backedge so the accumulator
            // stays register-homed (no per-iter stack round-trip, which made
            // the vectorized find_max SLOWER than scalar).
            O::VecBroadcastI32x8 | O::VecMaxI32x8 => Some(5),
            O::VecZeroI32x4 | O::VecLoadI32x4 | O::VecAddI32x4 | O::VecMulI32x4 => Some(6),
            O::VecZeroI64x2 | O::VecLoadI64x2 | O::VecAddI64x2 | O::VecMulI64x2 => Some(7),
            // v12 Fix C: the widening reductions PRODUCE an I64x2 dest (the
            // new accumulator). Classifying them as class 7 lets the Copy-web
            // propagation connect the backedge Copy (dest=phi_acc,
            // src=widen_dest) so the phi-merge Copy does not strand the
            // accumulator (src_class=None → evicted as bad). Combined with
            // the legal_consumer entry above, the full I64x2 accumulator
            // web — VecZero → Copy(phi) → VecWiden → Copy(backedge) →
            // VecHorizontalAdd — stays classified and reaches the XMM
            // allocator (register-homed accumulator, no stack round-trip).
            O::VecWidenAddI32x4ToI64x2 | O::VecWidenMaskedAddI32x4ToI64x2 => Some(7),
            // Equal-width masked add consumes an I32x8 accumulator exactly
            // like VecAddI32x8 (its lowering confines scratch to ymm0/ymm1,
            // mirroring the widening-masked path's xmm0/xmm1 discipline).
            O::VecMaskedAddI32x8 => Some(5),
            O::VecLoadWidenI32ToI64x2 => Some(7),
            _ => None,
        }
    };
    let legal_consumer = |op: &O, class: u8| -> bool {
        match class {
            1 => matches!(
                op,
                O::VecAddF32x8 | O::VecMulF32x8 | O::VecFmaF32x8 | O::VecHorizontalAddF32x8
            ),
            2 => matches!(
                op,
                O::VecAddF64x4 | O::VecMulF64x4 | O::VecFmaF64x4 | O::VecHorizontalAddF64x4
            ),
            3 => matches!(
                op,
                O::VecAddF32x4 | O::VecMulF32x4 | O::VecHorizontalAddF32x4
            ),
            4 => matches!(
                op,
                O::VecAddF64x2 | O::VecMulF64x2 | O::VecHorizontalAddF64x2
            ),
            5 => matches!(
                op,
                O::VecAddI32x8
                    | O::VecMulI32x8
                    | O::VecHorizontalAddI32x8
                    // v12 Fix F: the lane-wise max consumes the I32x8
                    // accumulator (read+write); the horizontal max reduces
                    // it. Both lowerings confine scratch to xmm0/xmm1.
                    | O::VecMaxI32x8
                    | O::VecHorizontalMaxI32x8
                    | O::VecMaskedAddI32x8
            ),
            6 => matches!(
                op,
                O::VecAddI32x4 | O::VecMulI32x4 | O::VecHorizontalAddI32x4
            ),
            7 => matches!(
                op,
                O::VecAddI64x2
                    | O::VecMulI64x2
                    | O::VecHorizontalAddI64x2
                    // v12 Fix C: the widening reductions consume an I64x2
                    // accumulator (read+write). Their lowerings (v12 Fix D)
                    // confine scratch to xmm0/xmm1, so a register-homed
                    // I64x2 accumulator is safe to keep across the widening
                    // step — whitelist them as legal consumers so the
                    // verification fixpoint does not evict the accumulator.
                    | O::VecWidenAddI32x4ToI64x2
                    | O::VecWidenMaskedAddI32x4ToI64x2
            ),
            _ => false,
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::InlineAsm { .. } | Instruction::Memcpy { .. } => {
                    return FxHashSet::default();
                }
                Instruction::Intrinsic { op, .. }
                    if class_of(op).is_none()
                        && !matches!(
                            op,
                            O::VecHorizontalAddF32x8
                                | O::VecHorizontalAddF64x4
                                | O::VecHorizontalAddF32x4
                                | O::VecHorizontalAddF64x2
                                | O::VecHorizontalAddI32x8
                                | O::VecHorizontalAddI32x4
                                | O::VecHorizontalAddI64x2
                                // v12 Fix C: widening reductions consume an
                                // I64x2 accumulator (whitelisted above). Their
                                // lowerings confine scratch to xmm0/xmm1 so an
                                // XMM-homed accumulator is safe — do NOT let
                                // them poison the whole function.
                                | O::VecWidenAddI32x4ToI64x2
                                | O::VecWidenMaskedAddI32x4ToI64x2
                                // v12 Fix C: broadcast/store intrinsics do
                                // not themselves need an accumulator, but
                                // they appear in the SAME function as
                                // reductions (e.g. loop_patterns' main has
                                // scale_add's VecBroadcastI32x8 +
                                // VecStoreI32x8 alongside sum_positive's
                                // widening reduction). Exempt them from the
                                // poison so the reduction's accumulator stays
                                // register-homed; the verification fixpoint
                                // evicts any genuine conflict (a store-feeding
                                // temp that overlaps the accumulator's live
                                // range).
                                | O::VecBroadcastI32x8
                                | O::VecBroadcastI32x4
                                | O::VecBroadcastF32x8
                                | O::VecBroadcastF32x4
                                | O::VecBroadcastF64x4
                                | O::VecBroadcastF64x2
                                | O::VecBroadcastI64x2
                                | O::VecStoreI32x8
                                | O::VecStoreI32x4
                                | O::VecStoreF32x8
                                | O::VecStoreF32x4
                                | O::VecStoreF64x4
                                | O::VecStoreF64x2
                                | O::VecStoreI64x2
                                // v12 Fix C: max reductions (find_max) and
                                // their horizontal reduce — lowerings
                                // confine scratch to xmm0/xmm1, exempt them.
                                | O::VecMaxI32x8
                                | O::VecHorizontalMaxI32x8
                                | O::VecHorizontalMaxI32x4
                                | O::VecSmaxI32x4
                                | O::VecLoadWidenI32ToI64x2
                        ) =>
                {
                    return FxHashSet::default();
                }
                _ => {}
            }
        }
    }

    let mut classes: FxHashMap<u32, u8> = FxHashMap::default();
    let mut conflicts: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest: Some(d), op, ..
            } = inst
            {
                if let Some(class) = class_of(op) {
                    if classes.insert(d.0, class).is_some_and(|old| old != class) {
                        conflicts.insert(d.0);
                    }
                }
            }
        }
    }

    // Phi-elim loop-carried vectors are Copy webs. Worklist, class-agreeing.
    let succs = copy_successors(func);
    let mut work: Vec<u32> = classes.keys().copied().collect();
    while let Some(src) = work.pop() {
        let Some(&class) = classes.get(&src) else {
            continue;
        };
        if let Some(ds) = succs.get(&src) {
            for &d in ds {
                match classes.get(&d) {
                    Some(&old) if old != class => {
                        conflicts.insert(d);
                    }
                    None => {
                        classes.insert(d, class);
                        work.push(d);
                    }
                    _ => {}
                }
            }
        }
    }
    for value in &conflicts {
        classes.remove(value);
    }

    loop {
        let mut bad: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                let mut allowed: FxHashSet<u32> = FxHashSet::default();
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => {
                        if classes.get(&dest.0) == classes.get(&src.0)
                            && classes.contains_key(&src.0)
                        {
                            allowed.insert(src.0);
                        }
                        if classes.contains_key(&dest.0) && !allowed.contains(&src.0) {
                            bad.insert(dest.0);
                        }
                    }
                    Instruction::Intrinsic { op, args, .. } => {
                        for arg in args {
                            if let Operand::Value(v) = arg {
                                if classes.get(&v.0).is_some_and(|&w| legal_consumer(op, w)) {
                                    allowed.insert(v.0);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        if classes.contains_key(&v.0) && !allowed.contains(&v.0) {
                            bad.insert(v.0);
                        }
                    }
                });
                for_each_value_use_in_instruction(inst, |v| {
                    if classes.contains_key(&v.0) && !allowed.contains(&v.0) {
                        bad.insert(v.0);
                    }
                });
            }
            for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    if classes.contains_key(&v.0) {
                        bad.insert(v.0);
                    }
                }
            });
        }
        if bad.is_empty() {
            break;
        }
        let old_len = classes.len();
        for value in bad {
            classes.remove(&value);
        }
        if classes.len() == old_len {
            break;
        }
    }

    // Keep only the loop-carried Copy web. Single-use loads/muls already
    // use the deferred-register path; allocating each independently adds a
    // YMM-to-YMM move after every producer.
    let mut copy_web: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            {
                if classes.get(&dest.0) == classes.get(&src.0) && classes.contains_key(&src.0) {
                    copy_web.insert(dest.0);
                    copy_web.insert(src.0);
                }
            }
        }
    }
    classes.retain(|value, _| copy_web.contains(value));
    classes.into_keys().collect()
}

/// Loop-invariant map broadcasts whose only uses are same-width packed
/// arithmetic. Transient map loads stay in the emitter's `%ymm0` deferred
/// chain; these live across every iteration.
fn collect_x86_map_broadcast_values(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp as O;

    let class_of = |op: &O| -> Option<u8> {
        match op {
            O::VecBroadcastF32x8 => Some(1),
            O::VecBroadcastF64x4 => Some(2),
            O::VecBroadcastI32x8 => Some(3),
            O::VecBroadcastF32x4 => Some(4),
            O::VecBroadcastF64x2 => Some(5),
            O::VecBroadcastI32x4 => Some(6),
            _ => None,
        }
    };
    let legal_consumer = |op: &O, class: u8| -> bool {
        match class {
            1 => matches!(op, O::VecMulF32x8 | O::VecAddF32x8 | O::VecMaddF32x8),
            2 => matches!(op, O::VecMulF64x4 | O::VecAddF64x4 | O::VecMaddF64x4),
            3 => matches!(op, O::VecMulI32x8 | O::VecAddI32x8),
            4 => matches!(op, O::VecMulF32x4 | O::VecAddF32x4),
            5 => matches!(op, O::VecMulF64x2 | O::VecAddF64x2),
            6 => matches!(op, O::VecMulI32x4 | O::VecAddI32x4),
            _ => false,
        }
    };

    let mut candidates: FxHashMap<u32, u8> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest: Some(d), op, ..
            } = inst
            {
                if let Some(class) = class_of(op) {
                    candidates.insert(d.0, class);
                }
            }
        }
    }

    let mut bad = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |operand| {
                let Operand::Value(value) = operand else {
                    return;
                };
                let Some(&class) = candidates.get(&value.0) else {
                    return;
                };
                let legal =
                    matches!(inst, Instruction::Intrinsic { op, .. } if legal_consumer(op, class));
                if !legal {
                    bad.insert(value.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                if candidates.contains_key(&v.0) {
                    // Address-side / pointer-only use: not a packed consumer.
                    let legal = matches!(
                        inst,
                        Instruction::Intrinsic { op, .. }
                            if candidates.get(&v.0).is_some_and(|&c| legal_consumer(op, c))
                    );
                    if !legal {
                        bad.insert(v.0);
                    }
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |operand| {
            if let Operand::Value(value) = operand {
                if candidates.contains_key(&value.0) {
                    bad.insert(value.0);
                }
            }
        });
    }
    candidates.retain(|value, _| !bad.contains(value));
    candidates.into_keys().collect()
}

/// 128/256-bit vector SSA values for the AArch64 NEON pool (40..47 → v16..v23).
fn collect_vector_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut vector_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest: Some(d), op, ..
            } = inst
            {
                if op.produces_vector_value() {
                    vector_values.insert(d.0);
                }
            }
        }
    }
    let succs = copy_successors(func);
    propagate_copy_web(&succs, &mut vector_values);
    vector_values
}

#[inline]
fn is_scalar_fp(ty: &IrType) -> bool {
    matches!(ty, IrType::F32 | IrType::F64)
}

/// Scalar F32/F64 SSA values. Phase 3 trusts this set exclusively (plus
/// `vector_values`) — ParamRef / Select / Call / F32 intrinsics MUST appear
/// here or they never get an XMM/v-register.
///
/// Copy propagation is what makes loop-carried FP accumulators (Copy form
/// after phi elimination) visible to the scan.
fn collect_f64_values(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp as O;

    let mut f64_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. }
                    if is_scalar_fp(ty) =>
                {
                    f64_values.insert(dest.0);
                }
                Instruction::Cast { dest, to_ty, .. } if is_scalar_fp(to_ty) => {
                    f64_values.insert(dest.0);
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        if is_scalar_fp(&info.return_type) {
                            f64_values.insert(dest.0);
                        }
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::F64(_)),
                }
                | Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::F32(_)),
                } => {
                    f64_values.insert(dest.0);
                }
                Instruction::Intrinsic {
                    dest: Some(d), op, ..
                } if matches!(
                    op,
                    O::SqrtF64
                        | O::FabsF64
                        | O::SqrtF32
                        | O::FabsF32
                        | O::FmaScalarF64
                        | O::FmaScalarF32
                        | O::RoundScalarF64(_)
                        | O::RoundScalarF32(_)
                        | O::CopysignF64
                        | O::CopysignF32
                ) =>
                {
                    f64_values.insert(d.0);
                }
                _ => {}
            }
        }
    }
    let succs = copy_successors(func);
    propagate_copy_web(&succs, &mut f64_values);
    f64_values
}

/// Strip values used as operands of instructions whose codegen still goes
/// through `resolve_slot_addr()` (not register-aware).
fn remove_ineligible_operands(
    func: &IrFunction,
    eligible: &mut FxHashSet<u32>,
    config: &RegAllocConfig,
) {
    let param_refs: FxHashSet<u32> = func
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .filter_map(|inst| match inst {
            Instruction::ParamRef { dest, .. } => Some(dest.0),
            _ => None,
        })
        .collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::CallIndirect {
                    func_ptr: Operand::Value(v),
                    ..
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::Memcpy { dest, src, .. } => {
                    // Direct ParamRef pointers are already in ABI registers and
                    // every backend's memcpy setup checks register assignments.
                    // Keeping them eligible removes entry spills/reloads for
                    // wrapper copies such as `*dst = *src`.
                    if !param_refs.contains(&dest.0) {
                        eligible.remove(&dest.0);
                    }
                    if !param_refs.contains(&src.0) {
                        eligible.remove(&src.0);
                    }
                }
                Instruction::VaArg { va_list_ptr, .. }
                | Instruction::VaStart { va_list_ptr }
                | Instruction::VaEnd { va_list_ptr } => {
                    eligible.remove(&va_list_ptr.0);
                }
                Instruction::VaCopy { dest_ptr, src_ptr } => {
                    eligible.remove(&dest_ptr.0);
                    eligible.remove(&src_ptr.0);
                }
                Instruction::VaArgStruct {
                    dest_ptr,
                    va_list_ptr,
                    ..
                } => {
                    eligible.remove(&dest_ptr.0);
                    eligible.remove(&va_list_ptr.0);
                }
                Instruction::AtomicRmw {
                    ptr: Operand::Value(v),
                    ..
                }
                | Instruction::AtomicInc {
                    ptr: Operand::Value(v),
                    ..
                }
                | Instruction::AtomicCmpxchg {
                    ptr: Operand::Value(v),
                    ..
                }
                | Instruction::AtomicLoad {
                    ptr: Operand::Value(v),
                    ..
                }
                | Instruction::AtomicStore {
                    ptr: Operand::Value(v),
                    ..
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::StackRestore { ptr } => {
                    eligible.remove(&ptr.0);
                }
                Instruction::InlineAsm {
                    outputs, inputs, ..
                } => {
                    if !config.allow_inline_asm_regalloc {
                        for (_, val, _) in outputs {
                            eligible.remove(&val.0);
                        }
                        for (_, op, _) in inputs {
                            if let Operand::Value(v) = op {
                                eligible.remove(&v.0);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Debug helper: short kind name of an instruction (CCC_DEBUG_HAZARDS).
fn inst_kind_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::BinOp { .. } => "BinOp",
        Instruction::Cmp { .. } => "Cmp",
        Instruction::Load { .. } => "Load",
        Instruction::Store { .. } => "Store",
        Instruction::Copy { .. } => "Copy",
        Instruction::Cast { .. } => "Cast",
        Instruction::Phi { .. } => "Phi",
        Instruction::Select { .. } => "Select",
        Instruction::ParamRef { .. } => "ParamRef",
        Instruction::GetElementPtr { .. } => "GEP",
        Instruction::Alloca { .. } => "Alloca",
        Instruction::GlobalAddr { .. } => "GlobalAddr",
        Instruction::LabelAddr { .. } => "LabelAddr",
        Instruction::Call { .. } => "Call",
        Instruction::CallIndirect { .. } => "CallIndirect",
        Instruction::InlineAsm { .. } => "InlineAsm",
        Instruction::UnaryOp { .. } => "UnaryOp",
        Instruction::StackRestore { .. } => "StackRestore",
        Instruction::Memcpy { .. } => "Memcpy",
        Instruction::Intrinsic { .. } => "Intrinsic",
        Instruction::PgoCounterInc { .. } => "PgoCounterInc",
        _ => "Other",
    }
}

/// i686 scratch-hazard scan for the ecx/edx caller-saved pool.
///
/// Returns, for (%ecx, %edx), the sorted list of program points at which
/// the emitted code may clobber it. WHITELIST: every instruction is a
/// hazard for both unless the match arm proves otherwise (fail-closed).
///
/// Part 1 consumes these with `overlaps_inclusive` (the insn at P may
/// clobber while still reading the value). Do not reuse `spans_any_call`.
fn collect_i686_scratch_hazard_points(
    func: &IrFunction,
    wide: &FxHashSet<u32>,
    ecx_clean_load_ptrs: &FxHashSet<u32>,
) -> (Vec<u32>, Vec<u32>) {
    collect_i686_scratch_hazard_points_refined(func, wide, ecx_clean_load_ptrs, None)
}

/// Assignment-aware hazard refinement (Phase 2h). `assignments` carries the
/// CURRENT register homes; when present, two conservative-hazard classes are
/// refined:
///
/// (a) Div/Rem with a register-homed divisor (∉ {edx,eax} — div_rhs_direct_ref's
///     contract): the emitter reads the divisor directly (`divl %reg`), so the
///     point is %ecx-clean. Constant divisors always stage through %ecx
///     (-Os) or take the magic-number path (-O2+) — never clean.
/// (b) gpr32 Loads whose pointer is a GetElementPtr: %edx-clean. Both the
///     folded indexed form (a single instruction) and the GEP-materialising
///     fallback (staging through %eax/%ecx) leave %edx untouched — unless
///     the load's DEST is homed in %edx (the store-to-home writes it).
fn collect_i686_scratch_hazard_points_refined(
    func: &IrFunction,
    wide: &FxHashSet<u32>,
    ecx_clean_load_ptrs: &FxHashSet<u32>,
    assignments: Option<&FxHashMap<u32, PhysReg>>,
) -> (Vec<u32>, Vec<u32>) {
    use crate::ir::reexports::{IrBinOp, IrUnaryOp};
    let mut ecx: Vec<u32> = Vec::new();
    let mut edx: Vec<u32> = Vec::new();
    let mut point: u32 = 0;

    // Same-block div/rem pair tails emit no code (the head stored both
    // results); see compute_i686_divrem_pairs for the soundness model.
    // Constant-RHS pairs fold at the head with the same clobber set as a
    // staged `divl`, so the model is exact for them too.
    let divrem_pairs = match divrem_target_for_current_arch() {
        Some(t) => compute_i686_divrem_pairs(func, t),
        None => I686DivRemPairs {
            tail_dests: Default::default(),
            head_partners: Default::default(),
            tail_points: Default::default(),
            head_point_of_tail: Default::default(),
        },
    };

    // Values defined by a GetElementPtr (refinement (b)).
    let mut gep_defined: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr { dest, .. } = inst {
                gep_defined.insert(dest.0);
            }
        }
    }

    // Allocas with alignment ≤ 16 resolve to SlotAddr::Direct.
    // Mirrors slot_assignment.rs: alloca_alignments only records > 16.
    let mut direct_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, align, .. } = inst {
                if *align <= 16 {
                    direct_allocas.insert(dest.0);
                }
            }
        }
    }

    let is_gpr32 = |ty: &IrType| {
        matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::Ptr
        )
    };
    let const_imm = |op: &Operand| {
        matches!(
            op,
            Operand::Const(IrConst::I8(_))
                | Operand::Const(IrConst::I16(_))
                | Operand::Const(IrConst::I32(_))
                | Operand::Const(IrConst::Zero)
        ) || matches!(op, Operand::Const(IrConst::I64(v)) if *v >= i32::MIN as i64 && *v <= i32::MAX as i64)
    };

    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            // Fused same-block div/rem pairs: the TAIL instruction emits
            // nothing (its result was stored by the head's dual-store
            // emission earlier in the block), so it is a hazard for neither
            // %ecx nor %edx. Constant-RHS pairs are fused by the head's
            // constant fold (one magic sequence yields both results), so
            // the tail is clean for them as well; the head keeps the
            // "constant divisor: never clean" rule below.
            let divrem_tail = divrem_pairs.tail_points.contains(&(bi, ii));
            let (ecx_clean, edx_clean) = match inst {
                Instruction::BinOp { op, ty, rhs, .. }
                    if is_gpr32(ty)
                        && divrem_tail
                        && matches!(
                            op,
                            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
                        ) =>
                {
                    (true, true)
                }
                Instruction::BinOp { op, ty, rhs, .. } if is_gpr32(ty) => match op {
                    IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::Mul
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Shl
                    | IrBinOp::AShr
                    | IrBinOp::LShr => (const_imm(rhs), true),
                    IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => {
                        // Refinement (a): a register-homed divisor (∉
                        // {edx,eax}) is read directly (`divl %reg`) — the
                        // point is %ecx-clean. %edx always receives the
                        // remainder/quotient write. Constant divisors stage
                        // through %ecx (or go magic) — never clean.
                        let direct_divisor = match (rhs, assignments) {
                            (Operand::Value(rv), Some(asg)) => {
                                asg.get(&rv.0).is_some_and(|&p| p.0 != 5 && p.0 != 6)
                            }
                            _ => false,
                        };
                        (direct_divisor, false)
                    }
                    _ => (false, false),
                },
                Instruction::Cmp { ty, rhs, .. } if is_gpr32(ty) => (const_imm(rhs), true),
                // gpr32↔gpr32 casts stage only through %eax: the same-size
                // kinds are pure no-ops (Noop / UnsignedToSignedSameSize emit
                // nothing; coalesced members never even reach here) and the
                // narrowing kinds are sub-register movs{x}l/movz{x}l on %eax,
                // all wrapped in operand_to_eax/store_eax_to. The old
                // catch-all (false, false) marked vsprintf number()'s two
                // U32→I32 casts as %edx hazards inside the digit loop and
                // kept the URem remainder (the folded load's natural index,
                // born in %edx at the fused div) out of its own birth
                // register.
                Instruction::Cast { from_ty, to_ty, .. }
                    if is_gpr32(from_ty) && is_gpr32(to_ty) =>
                {
                    (true, true)
                }
                Instruction::Copy { dest, src } => match src {
                    Operand::Value(v) => {
                        let c = !wide.contains(&v.0) && !wide.contains(&dest.0);
                        (c, c)
                    }
                    c => {
                        let k = const_imm(c) && !wide.contains(&dest.0);
                        (k, k)
                    }
                },
                Instruction::UnaryOp { op, ty, .. }
                    if is_gpr32(ty)
                        && matches!(
                            op,
                            IrUnaryOp::Neg
                                | IrUnaryOp::Not
                                | IrUnaryOp::Clz
                                | IrUnaryOp::Ctz
                                | IrUnaryOp::Bswap
                                | IrUnaryOp::Popcount
                        ) =>
                {
                    (true, true)
                }
                Instruction::Load {
                    ty,
                    ptr,
                    dest,
                    seg_override,
                    ..
                } if is_gpr32(ty) => {
                    // A Load stages its pointer through %ecx ONLY when the
                    // pointer has no register home (it must be loaded into a
                    // scratch to dereference).  When the pointer value is
                    // register-resident — or never materialised at all
                    // (folded absolute global) — the emitter uses direct
                    // `movX (%ptr),…` / absolute addressing and never touches
                    // %ecx.  First pass conservatively assumes slot-resident
                    // pointers; the Phase-2d refinement re-runs with the
                    // actually-assigned pointer set (see allocate_registers).
                    let clean = (direct_allocas.contains(&ptr.0)
                        || ecx_clean_load_ptrs.contains(&ptr.0))
                        && *seg_override == crate::common::types::AddressSpace::Default;
                    // Refinement (b): a GEP-pointer load leaves %edx
                    // untouched — the folded indexed form is a single
                    // instruction and the GEP-materialising fallback stages
                    // through %eax/%ecx — unless the load's DEST is homed in
                    // %edx (the store-to-home writes it).
                    let edx_clean = if gep_defined.contains(&ptr.0) {
                        match assignments {
                            Some(asg) => asg.get(&dest.0) != Some(&PhysReg(5)),
                            None => false, // dest homes unknown: conservative
                        }
                    } else {
                        true
                    };
                    (clean, edx_clean)
                }
                Instruction::Store {
                    ty,
                    ptr,
                    seg_override,
                    ..
                } => {
                    let direct32 = is_gpr32(ty)
                        && direct_allocas.contains(&ptr.0)
                        && *seg_override == crate::common::types::AddressSpace::Default;
                    (direct32, direct32)
                }
                Instruction::GetElementPtr { .. } => (false, true),
                Instruction::Alloca { .. }
                | Instruction::Phi { .. }
                | Instruction::PgoCounterInc { .. } => (true, true),
                Instruction::Select { ty, .. } if is_gpr32(ty) => (true, true),
                Instruction::ParamRef { ty, .. } if is_gpr32(ty) => (true, true),
                Instruction::GlobalAddr { .. } | Instruction::LabelAddr { .. } => (true, true),
                _ => (false, false),
            };
            if !ecx_clean {
                ecx.push(point);
                if env_on("CCC_DEBUG_HAZARDS") {
                    eprintln!(
                        "[HZ] fn={} pt={} ECX-DIRTY {:?}",
                        func.name,
                        point,
                        inst_kind_name(inst)
                    );
                }
            }
            if !edx_clean {
                edx.push(point);
                if env_on("CCC_DEBUG_HAZARDS") {
                    eprintln!(
                        "[HZ] fn={} pt={} EDX-DIRTY {:?}",
                        func.name,
                        point,
                        inst_kind_name(inst)
                    );
                }
            }
            point += 1;
        }
        let ret_is_gpr32 = is_gpr32(&func.return_type);
        let (t_ecx_clean, t_edx_clean) = match &block.terminator {
            Terminator::Return(Some(_)) => (true, ret_is_gpr32),
            Terminator::Return(None) => (true, true),
            Terminator::Branch(_) => (true, true),
            // Compare-and-branch fusion re-emits the block's Cmp here.
            Terminator::CondBranch { .. } => (false, true),
            Terminator::Switch { .. } => (false, true),
            Terminator::IndirectBranch { .. } => (false, false),
            Terminator::Unreachable => (true, true),
        };
        if !t_ecx_clean {
            ecx.push(point);
            if env_on("CCC_DEBUG_HAZARDS") {
                eprintln!("[HZ] fn={} pt={} ECX-DIRTY term", func.name, point);
            }
        }
        if !t_edx_clean {
            edx.push(point);
            if env_on("CCC_DEBUG_HAZARDS") {
                eprintln!("[HZ] fn={} pt={} EDX-DIRTY term", func.name, point);
            }
        }
        point += 1;
    }
    (ecx, edx)
}

/// i686 %eax-as-home hazard scan (Phase 2e).
///
/// The accumulator is a valid register HOME only across program points where
/// the emitted code provably does not use %eax as scratch. On i686 that set
/// is tiny — every binop/cast/load/store/call/div/asm stages through %eax —
/// so this is a WHITELIST of the emitters known to leave %eax untouched:
///   * `Phi`               — emits no code;
///   * `Branch`/`Unreachable` terminators — emit no code.
/// Everything else (including `Return` — its operand staging writes %eax —
/// `CondBranch`/`Switch` condition consumption, copies, GEP/GlobalAddr/
/// LabelAddr materialisation, and all ALU/memory ops) is a hazard point.
/// A value homed in %eax therefore only spans straight-line phi/branch
/// corridors between its (excluded) definition point and its use.
///
/// Point numbering MUST match collect_i686_scratch_hazard_points (and thus
/// the liveness intervals): one point per instruction, one per terminator.
fn collect_i686_eax_hazard_points(func: &IrFunction) -> Vec<u32> {
    let mut hazards: Vec<u32> = Vec::new();
    let mut point: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if !matches!(inst, Instruction::Phi { .. }) {
                hazards.push(point);
            }
            point += 1;
        }
        if !matches!(
            &block.terminator,
            Terminator::Branch(_) | Terminator::Unreachable
        ) {
            hazards.push(point);
        }
        point += 1;
    }
    hazards
}

/// Exclude every 3rd fusible multiply temp from register allocation.
///
/// x86-64 only (Part 1 gates on the xmm-20 pool):
/// - Channel 1/2: register-allocated temps
/// - Channel 3: accumulator path (%eax) via mul-add fusion
///
/// Under register pressure this can add spills; do not port off x86.
/// Recount is *unweighted* on purpose: Part 1's `use_count` is loop-
/// weighted and would hide the single-use fusion predicate.
fn exclude_every_third_mul_temp(func: &IrFunction, eligible: &mut FxHashSet<u32>) {
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    let mut fusible_temps: Vec<u32> = Vec::new();
    for block in &func.blocks {
        for (idx, inst) in block.instructions.iter().enumerate() {
            let (mul_dest, mul_ty) = match inst {
                Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Mul,
                    ty,
                    ..
                } => (dest, ty),
                _ => continue,
            };
            if mul_ty.is_float() || matches!(mul_ty, IrType::I128 | IrType::U128) {
                continue;
            }
            if use_count.get(&mul_dest.0).copied().unwrap_or(0) != 1 {
                continue;
            }
            if let Some(Instruction::BinOp {
                op: crate::ir::reexports::IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            }) = block.instructions.get(idx + 1)
            {
                let mul_is_operand = matches!(lhs, Operand::Value(v) if v.0 == mul_dest.0)
                    || matches!(rhs, Operand::Value(v) if v.0 == mul_dest.0);
                if mul_is_operand && mul_ty == add_ty {
                    fusible_temps.push(mul_dest.0);
                }
            }
        }
    }

    if fusible_temps.len() < 6 {
        return;
    }
    for (i, &temp_id) in fusible_temps.iter().enumerate() {
        if i % 3 == 2 {
            eligible.remove(&temp_id);
        }
    }
}

/// A phi/backedge pair that may share one physical register.
///
/// `source_def_idx..copy_idx` is a straight-line window when
/// `source_block_idx == block_idx`.  When the source is defined in the unique
/// predecessor of `block_idx`, the window is `(source_def_idx..end_of_pred) +
/// (start_of_block..copy_idx)`.  This covers branch-separated latch copies from
/// Backedge PRE without allowing arbitrary cross-block destructive updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhiCoalesceCandidate {
    pub(crate) phi_dest: u32,
    pub(crate) backedge_src: u32,
    pub(crate) block_idx: usize,
    pub(crate) source_block_idx: usize,
    pub(crate) source_def_idx: usize,
    pub(crate) copy_idx: usize,
}

/// True iff `inst` clobbers the caller-saved GPR set. Mirrors the
/// conservative core of `liveness::instruction_is_call_point` without
/// pulling in the i686-libcall / F128 special cases: a phi-destructive
/// update across any of these is only legal in a callee-saved home.
#[inline]
fn instruction_clobbers_caller_saved(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::Memcpy { .. }
            | Instruction::VaArg { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaCopy { .. }
            | Instruction::VaArgStruct { .. }
    )
}

/// The (def, copy) window of a phi-coalesce candidate contains a caller-saved
/// clobber. Sharing the dest register then means the source is born in that
/// register *before* the clobber and read after it — illegal for rdi/rsi/…
fn phi_window_clobbers_caller_saved(func: &IrFunction, cand: &PhiCoalesceCandidate) -> bool {
    let Some(copy_block) = func.blocks.get(cand.block_idx) else {
        return false;
    };
    let Some(src_block) = func.blocks.get(cand.source_block_idx) else {
        return false;
    };
    if cand.copy_idx > copy_block.instructions.len()
        || cand.source_def_idx >= src_block.instructions.len()
    {
        return false;
    }
    if cand.source_block_idx == cand.block_idx {
        if cand.source_def_idx >= cand.copy_idx {
            return false;
        }
        copy_block.instructions[cand.source_def_idx + 1..cand.copy_idx]
            .iter()
            .any(instruction_clobbers_caller_saved)
    } else {
        src_block.instructions[cand.source_def_idx + 1..]
            .iter()
            .chain(copy_block.instructions[..cand.copy_idx].iter())
            .any(instruction_clobbers_caller_saved)
    }
}

/// Revalidate the same-block destructive-update proof and propagate an
/// assigned phi register to its backedge producer. Part 1 calls this after
/// the GPR scan (`apply_phi_coalesce_assignments(..., &iv_map, ...)`).
///
/// `iv_map` is the O(1) `[start, end]` index. Conflict checks still walk
/// `liveness.intervals` so a multi-segment *other* value cannot hide an
/// overlap behind iv_map's last-write-wins.
///
/// `callee_saved` is the Phase-1 pool. A source whose window contains a
/// caller-saved clobber may inherit the dest home only when that home is
/// in this pool (the call preserves it).
fn apply_phi_coalesce_assignments(
    func: &IrFunction,
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    candidates: &[PhiCoalesceCandidate],
    assignments: &mut FxHashMap<u32, PhysReg>,
    callee_saved: &[PhysReg],
) {
    for candidate in candidates {
        let phi_dest = candidate.phi_dest;
        let backedge_src = candidate.backedge_src;
        let Some(copy_block) = func.blocks.get(candidate.block_idx) else {
            continue;
        };
        let Some(src_block) = func.blocks.get(candidate.source_block_idx) else {
            continue;
        };
        if candidate.copy_idx >= copy_block.instructions.len()
            || candidate.source_def_idx >= src_block.instructions.len()
            || src_block.instructions[candidate.source_def_idx]
                .dest()
                .is_none_or(|dest| dest.0 != backedge_src)
            || !matches!(
                copy_block.instructions[candidate.copy_idx],
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } if dest.0 == phi_dest && src.0 == backedge_src
            )
        {
            continue;
        }
        let phi_used_in_window = if candidate.source_block_idx == candidate.block_idx {
            candidate.source_def_idx >= candidate.copy_idx
                || copy_block.instructions[candidate.source_def_idx + 1..candidate.copy_idx]
                    .iter()
                    .any(|inst| uses_value(inst, phi_dest))
        } else {
            src_block.instructions[candidate.source_def_idx + 1..]
                .iter()
                .chain(copy_block.instructions[..candidate.copy_idx].iter())
                .any(|inst| uses_value(inst, phi_dest))
        };
        if phi_used_in_window {
            continue;
        }

        let Some(&reg) = assignments.get(&phi_dest) else {
            continue;
        };

        if phi_window_clobbers_caller_saved(func, candidate)
            && !callee_saved.iter().any(|r| r.0 == reg.0)
        {
            if env_on("CCC_DEBUG_PHI_COALESCE") {
                eprintln!(
                    "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{} r{}: call in window",
                    phi_dest, backedge_src, reg.0
                );
            }
            continue;
        }

        if let Some(&src_iv) = iv_map.get(&backedge_src) {
            let has_conflict = liveness.intervals.iter().any(|iv| {
                if iv.value_id == backedge_src || iv.value_id == phi_dest {
                    return false;
                }
                assignments.get(&iv.value_id).is_some_and(|other_reg| {
                    other_reg.0 == reg.0 && intervals_overlap((iv.start, iv.end), src_iv)
                })
            });
            if has_conflict {
                continue;
            }
        }
        assignments.insert(backedge_src, reg);
    }
}

/// Detect safe phi-coalesce candidates for loop-carried variables.
///
/// After phi elimination a backedge contains `%phi = copy %next`. Sharing a
/// register removes that copy, but it is a destructive update: `%next`'s
/// definition overwrites `%phi`. Safe only when definition and Copy are in
/// the SAME basic block and `%phi` is not read between them (SQLite
/// `deleteTable`: a source defined in an earlier block can still be live
/// into an intervening successor).
///
/// Also used by stack-layout copy coalescing. Signature is part of that
/// contract — do not change it.
///
/// ALL pairs are returned. Part 1 takes one pair per dest (first wins);
/// the slot coalescer uses `claimed_dests`. Candidates are sorted so the
/// hottest latch (deeper loop, later copy) comes first — that is what
/// makes Part 1's first-wins pick the backedge rather than the preheader
/// init (the gzip longest_match shuffle).
pub(crate) fn detect_phi_coalesce_groups(
    func: &IrFunction,
    liveness: &LivenessResult,
) -> Vec<PhiCoalesceCandidate> {
    // A phi-elim dest is a Copy destination with more than one definition
    // (any instruction, any block). Requiring Copies in *different* blocks
    // dropped the single-block `i = 0; loop: t = i+1; i = t;` latch —
    // the most common tight-loop shape.
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut copy_dest: FxHashSet<u32> = FxHashSet::default();
    let mut unique_def_site: FxHashMap<u32, Option<(usize, usize)>> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
                unique_def_site
                    .entry(dest.0)
                    .and_modify(|site| *site = None)
                    .or_insert(Some((block_idx, inst_idx)));
            }
            if let Instruction::Copy { dest, .. } = inst {
                copy_dest.insert(dest.0);
            }
        }
    }
    let mut multi_def: FxHashSet<u32> = FxHashSet::default();
    for &v in &copy_dest {
        if def_count.get(&v).copied().unwrap_or(0) > 1 {
            multi_def.insert(v);
        }
    }
    if multi_def.is_empty() {
        return Vec::new();
    }

    let mut src_use_blocks: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(value) = op {
                    src_use_blocks.entry(value.0).or_default().insert(block_idx);
                }
            });
            for_each_value_use_in_instruction(inst, |value| {
                src_use_blocks.entry(value.0).or_default().insert(block_idx);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(value) = op {
                src_use_blocks.entry(value.0).or_default().insert(block_idx);
            }
        });
    }

    let label_to_idx = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_to_idx);

    let debug = env_on("CCC_DEBUG_PHI_COALESCE");
    let mut candidates = Vec::new();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        if liveness
            .block_loop_depth
            .get(block_idx)
            .copied()
            .unwrap_or(0)
            == 0
        {
            continue;
        }

        for (copy_idx, inst) in block.instructions.iter().enumerate() {
            let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            else {
                continue;
            };
            if !multi_def.contains(&dest.0) || multi_def.contains(&src.0) {
                continue;
            }

            let Some((source_block, source_def_idx)) =
                unique_def_site.get(&src.0).copied().flatten()
            else {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}): source is not single-def",
                        dest.0, src.0
                    );
                }
                continue;
            };

            let same_block = source_block == block_idx;
            let source_is_fp = func.blocks[source_block].instructions[source_def_idx]
                .result_type()
                .is_some_and(|ty| ty.is_float());
            let pred_is_unique = !same_block
                && source_is_fp
                && preds.len(block_idx) == 1
                && preds.row(block_idx)[0] as usize == source_block
                // Do not coalesce a preheader/init definition into a loop phi:
                // the source must be born on the same loop-carried path, not
                // before the loop. Backedge PRE's branch-separated latch shape
                // has equal loop depth for source and copy blocks; the
                // 20041011-1 torture failure exposed the preheader case.
                && liveness.block_loop_depth.get(source_block).copied().unwrap_or(0)
                    >= liveness.block_loop_depth.get(block_idx).copied().unwrap_or(0);
            if !(same_block && source_def_idx < copy_idx || pred_is_unique) {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}): def block/index {}:{} cannot feed copy {}:{}",
                        dest.0, src.0, source_block, source_def_idx, block_idx, copy_idx
                    );
                }
                continue;
            }

            let phi_used_in_window = if same_block {
                block.instructions[source_def_idx + 1..copy_idx]
                    .iter()
                    .any(|middle| uses_value(middle, dest.0))
            } else {
                func.blocks[source_block].instructions[source_def_idx + 1..]
                    .iter()
                    .chain(block.instructions[..copy_idx].iter())
                    .any(|middle| uses_value(middle, dest.0))
            };
            // A use can be hidden behind a value computed before the destructive
            // update.  In `p = &a[i]; next = i + 1; *p = ...; i = next`, the
            // Store reads `p`, not `i`, so the direct test above misses it.
            // x86 can defer/fold that GEP into the Store's SIB operand; sharing
            // the homes then increments `i` before the effective address is
            // formed.  Track the pre-update dataflow closure and reject when a
            // derived old-phi value remains live after the source definition.
            let phi_derived_used_in_window = if same_block {
                let mut derived: FxHashSet<u32> = FxHashSet::default();
                derived.insert(dest.0);
                for earlier in &block.instructions[..source_def_idx] {
                    let mut depends = false;
                    for_each_operand_in_instruction(earlier, |op| {
                        if matches!(op, Operand::Value(v) if derived.contains(&v.0)) {
                            depends = true;
                        }
                    });
                    for_each_value_use_in_instruction(earlier, |v| {
                        if derived.contains(&v.0) {
                            depends = true;
                        }
                    });
                    if depends {
                        if let Some(d) = earlier.dest() {
                            derived.insert(d.0);
                        }
                    }
                }
                block.instructions[source_def_idx + 1..copy_idx]
                    .iter()
                    .any(|middle| derived.iter().any(|&v| uses_value(middle, v)))
            } else {
                false
            };
            let source_used_elsewhere = src_use_blocks.get(&src.0).is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|&use_block| use_block != block_idx && use_block != source_block)
            });
            let source_used_before_copy = !same_block
                && block.instructions[..copy_idx]
                    .iter()
                    .any(|middle| uses_value(middle, src.0));
            if phi_used_in_window
                || phi_derived_used_in_window
                || source_used_elsewhere
                || source_used_before_copy
            {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}) block={} source_block={} used_in_window={} derived_in_window={} cross_block={} src_before_copy={}",
                        dest.0,
                        src.0,
                        block_idx,
                        source_block,
                        phi_used_in_window,
                        phi_derived_used_in_window,
                        source_used_elsewhere,
                        source_used_before_copy
                    );
                }
                continue;
            }

            if debug {
                eprintln!(
                    "[PHI_COALESCE] Coalescing phi_dest=Value({}) with backedge_src=Value({}) source block {} idx {} copy block {} idx {}",
                    dest.0, src.0, source_block, source_def_idx, block_idx, copy_idx
                );
            }
            candidates.push(PhiCoalesceCandidate {
                phi_dest: dest.0,
                backedge_src: src.0,
                block_idx,
                source_block_idx: source_block,
                source_def_idx,
                copy_idx,
            });
        }
    }

    // Hottest latch first: deeper loop, then later copy in the block
    // (latch sits after the body). Part 1 / claimed_dests first-wins.
    candidates.sort_by(|a, b| {
        let da = liveness
            .block_loop_depth
            .get(a.block_idx)
            .copied()
            .unwrap_or(0);
        let db = liveness
            .block_loop_depth
            .get(b.block_idx)
            .copied()
            .unwrap_or(0);
        db.cmp(&da)
            .then(a.copy_idx.cmp(&b.copy_idx).reverse())
            .then(a.phi_dest.cmp(&b.phi_dest))
    });

    candidates
}

/// True iff `inst` uses `val_id` as an operand (not as dest).
/// Canonical visitors cover Intrinsic args, Memcpy endpoints, InlineAsm
/// inputs, atomics — a hand-maintained match previously missed `Intrinsic`
/// args and let phi coalescing merge a pointer phi with its backedge
/// increment while the phi was still live as an intrinsic operand
/// (zlib-ng adler32_avx2: in-place `addq $32` clobbered the load address).
#[inline]
fn uses_value(inst: &Instruction, val_id: u32) -> bool {
    let mut found = false;
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            if v.0 == val_id {
                found = true;
            }
        }
    });
    if !found {
        for_each_value_use_in_instruction(inst, |v| {
            if v.0 == val_id {
                found = true;
            }
        });
    }
    found
}

#[cfg(test)]
mod phi_coalesce_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrCmpOp, Value};

    #[test]
    fn segment_interference_preserves_holes_and_half_open_handoffs() {
        assert!(!segment_sets_overlap(&[(1, 3), (8, 10)], &[(3, 8)]));
        assert!(!segment_sets_overlap(&[(1, 5)], &[(5, 9)]));
        assert!(segment_sets_overlap(&[(1, 5), (9, 12)], &[(4, 7)]));
        assert!(segment_sets_overlap(&[(1, 2), (6, 9)], &[(3, 7)]));

        let mut occupied = vec![(1, 3), (8, 10)];
        insert_segment_union(&mut occupied, &[(3, 5), (6, 8), (12, 14)]);
        assert_eq!(occupied, vec![(1, 5), (6, 10), (12, 14)]);
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn rejects_source_defined_before_intervening_phi_use_block() {
        // Reduced sqlite deleteTable shape:
        //   block 2 defines the proposed backedge source;
        //   block 3 still reads the old phi value;
        //   block 4 performs the backedge Copy.
        let mut func = IrFunction::new("deleteTable_shape".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                Vec::new(),
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(5),
                },
            ),
            block(
                2,
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(5)),
                    ty: IrType::I32,
                }],
                Terminator::Branch(BlockId(4)),
            ),
            block(
                4,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                5,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 4;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            candidates.is_empty(),
            "cross-block source must not coalesce: {candidates:?}"
        );
    }

    #[test]
    fn accepts_same_block_latch_copy() {
        // Tight do-while, init + latch Copies in ONE block:
        //   v1 = 0
        //   v2 = v1 + 1
        //   v1 = v2          ← must coalesce (old multi_def missed this)
        //   condbr v2, self, exit
        let mut func = IrFunction::new("same_block_latch".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I32(0)),
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(0),
                    false_label: BlockId(1),
                },
            ),
            block(
                1,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 3;

        let liveness = compute_live_intervals(&func);
        if liveness.block_loop_depth.first().copied().unwrap_or(0) == 0 {
            // Loop-depth oracle did not mark the self-backedge; the
            // detector correctly refuses depth-0 blocks.
            return;
        }
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "same-block latch must coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_phi_use_between_def_and_copy() {
        let mut func = IrFunction::new("window_use".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(5)),
                        ty: IrType::I32,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 4;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            !candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "window use of phi must block coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_phi_value_derived_before_update_and_used_after_it() {
        // Reduced from gcc.c-torture/execute/pr51933.c after GVN merged two
        // `i + 1` values.  The backend may fold the GEP into Store, so the
        // address dependency on old `i` remains live past the next-IV def.
        let mut func =
            IrFunction::new("deferred_gep_index".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(1)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    },
                    Instruction::GlobalAddr {
                        dest: Value(4),
                        name: "table".to_string(),
                    },
                    Instruction::GetElementPtr {
                        dest: Value(5),
                        base: Value(4),
                        offset: Operand::Value(Value(3)),
                        ty: IrType::Ptr,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I8(7)),
                        ptr: Value(5),
                        ty: IrType::I8,
                        seg_override: crate::common::types::AddressSpace::Default,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 6;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            !candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "derived old-phi address must block destructive coalescing: {candidates:?}"
        );
    }

    #[test]
    fn hottest_latch_sorts_first() {
        // Two latches on the same dest: an early copy and a later copy.
        // First-wins consumers must see the later (latch) pair first.
        let mut func = IrFunction::new("two_latches".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(3)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 4;

        let liveness = compute_live_intervals(&func);
        if liveness.block_loop_depth.get(1).copied().unwrap_or(0) == 0 {
            return;
        }
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        let dest1: Vec<_> = candidates.iter().filter(|c| c.phi_dest == 1).collect();
        if dest1.len() >= 2 {
            assert_eq!(
                dest1[0].backedge_src, 3,
                "later latch must sort first: {candidates:?}"
            );
        }
    }

    fn empty_call(dest: Option<Value>) -> Instruction {
        Instruction::Call {
            func: "strtol".to_string(),
            info: crate::ir::reexports::CallInfo {
                dest,
                ..Default::default()
            },
        }
    }

    /// `++i` then a call then `i = i1` must not put i1 in the dest's
    /// caller-saved home: the call clobbers it (loop_iv_across_call).
    #[test]
    fn refuses_caller_saved_home_across_call_in_window() {
        let mut func = IrFunction::new("iv_across_call".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(1)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    empty_call(None),
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 3;

        let liveness = compute_live_intervals(&func);
        let iv_map = interval_map(&liveness);
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 1,
            source_block_idx: 1,
            source_def_idx: 0,
            copy_idx: 2,
        };
        assert!(
            phi_window_clobbers_caller_saved(&func, &cand),
            "strtol in the (def, copy) window must be a caller-saved clobber"
        );

        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(14)); // %edi — caller-saved
        apply_phi_coalesce_assignments(
            &func,
            &liveness,
            &iv_map,
            &[cand],
            &mut assignments,
            &[PhysReg(1)], // only rbx is callee-saved
        );
        assert_eq!(
            assignments.get(&2),
            None,
            "call-spanning ++i must not inherit %edi: {assignments:?}"
        );

        // A callee-saved dest home is still legal: the call preserves it.
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        apply_phi_coalesce_assignments(
            &func,
            &liveness,
            &iv_map,
            &[cand],
            &mut assignments,
            &[PhysReg(1)],
        );
        assert_eq!(
            assignments.get(&2).copied(),
            Some(PhysReg(1)),
            "callee-saved home may still be shared across the call: {assignments:?}"
        );
    }

    /// sqlite3 vdbeChangeP4Full: `if (n==0) n = strlen(z); memcpy; p[n]=0`.
    /// The join Copy of I32 `n` is multi-def. `resolve_index` peels the
    /// widening Cast so folded_index_uses keys on that Copy dest, whose last
    /// IR use is the Cast — before memcpy — while the GEP (dest.start) sits
    /// after it. Multi-def required must start at idx_ir_end so the Copy dest
    /// is live across the call (callee-saved / spilled), not a clobbered %r10.
    #[test]
    fn multi_def_peeled_index_covers_call_before_gep() {
        let mut func =
            IrFunction::new("vdbe_strndup_shape".to_string(), IrType::Ptr, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Alloca {
                        dest: Value(10),
                        ty: IrType::I8,
                        size: 64,
                        align: 1,
                        volatile: false,
                        semantic_volatile: false,
                    },
                    Instruction::Copy {
                        dest: Value(0),
                        src: Operand::Const(IrConst::I32(5)),
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Eq,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(0)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    Instruction::Copy {
                        dest: Value(5),
                        src: Operand::Const(IrConst::I32(4)),
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(5)),
                    },
                ],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(1)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    },
                    Instruction::Memcpy {
                        dest: Value(10),
                        src: Value(10),
                        size: 8,
                    },
                    Instruction::GetElementPtr {
                        dest: Value(4),
                        base: Value(10),
                        offset: Operand::Value(Value(3)),
                        ty: IrType::Ptr,
                    },
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I8(0)),
                        ptr: Value(4),
                        ty: IrType::I8,
                        seg_override: crate::common::types::AddressSpace::Default,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            ),
        ];
        func.next_value_id = 11;

        let mut folded = FxHashMap::default();
        folded.insert(1u32, vec![4u32]);
        let config = RegAllocConfig {
            available_regs: vec![PhysReg(1), PhysReg(2), PhysReg(3), PhysReg(4), PhysReg(5)],
            accumulator_policy: AccumulatorPolicy {
                operand_order: AccumulatorOperandOrder::LhsFirst,
                return_consumes_accumulator: false,
            },
            caller_saved_regs: vec![
                PhysReg(10),
                PhysReg(11),
                PhysReg(12),
                PhysReg(13),
                PhysReg(14),
                PhysReg(15),
            ],
            call_arg_regs: vec![PhysReg(14), PhysReg(15), PhysReg(12), PhysReg(13)],
            indirect_target_regs: vec![PhysReg(11)],
            allow_inline_asm_regalloc: false,
            leaf_caller_saved_homes: false,
            xmm_regs: Vec::new(),
            never_materialized: FxHashSet::default(),
            folded_index_uses: folded,
            reg_hints: FxHashMap::default(),
        };
        let result = allocate_registers(&func, &config);
        let liv = result.liveness.expect("liveness");
        assert!(!liv.call_points.is_empty(), "Memcpy must be a call_point");
        let segs: Vec<(u32, u32)> = liv
            .segments
            .iter()
            .filter(|s| s.value_id == 1)
            .map(|s| (s.start, s.end))
            .collect();
        let covered = liv
            .call_points
            .iter()
            .any(|&cp| segs.iter().any(|&(s, e)| s <= cp && cp < e));
        assert!(
            covered,
            "peeled multi-def index v1 must stay live across memcpy; segs={segs:?} calls={:?}",
            liv.call_points
        );
        if let Some(home) = result.assignments.get(&1) {
            assert!(
                !matches!(home.0, 10 | 11 | 12 | 13 | 14 | 15 | 16),
                "index live across memcpy must not take a caller-saved home (got r{})",
                home.0
            );
        }
    }
}
