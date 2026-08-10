//! Linear scan register allocator.
//!
//! Assigns physical registers to IR values based on their live intervals.
//! Values with the longest live ranges and most uses get priority for register
//! assignment. Values that don't fit in available registers remain on the stack.
//!
//! Three-phase allocation:
//! 1. **Callee-saved registers** (x86: rbx, r12-r15; ARM: x20-x28; RISC-V: s1, s7-s11):
//!    Assigned to values whose live ranges span function calls. These registers
//!    are preserved across calls by the ABI, so no save/restore is needed at call
//!    sites (but prologue/epilogue must save them).
//!
//! 2. **Caller-saved registers** (x86: r11, r10, r8, r9; ARM: x13, x14):
//!    Assigned to values whose live ranges do NOT span any function call. These
//!    registers are destroyed by calls, so they can only hold values between calls.
//!    No prologue/epilogue save/restore is needed since we never assign them to
//!    values that cross call boundaries.
//!
//! 3. **Callee-saved spillover**: After phases 1 and 2, any remaining callee-saved
//!    registers are assigned to the highest-priority non-call-spanning values that
//!    didn't fit in the caller-saved pool. This is critical for call-free hot loops
//!    (e.g., hash functions, matrix multiply, sorting) where all values compete for
//!    only a few caller-saved registers. The one-time prologue/epilogue save/restore
//!    cost is amortized over many loop iterations.

use super::live_range::{self, LinearScanAllocator};
use super::liveness::{
    compute_live_intervals, for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction, LiveInterval, LivenessResult,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, Operand, Terminator};

/// A physical register assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysReg(pub u8);

/// Result of register allocation for a function.
pub struct RegAllocResult {
    /// Map from value ID -> assigned physical register.
    pub assignments: FxHashMap<u32, PhysReg>,
    /// Set of physical registers actually used (for prologue/epilogue save/restore).
    pub used_regs: Vec<PhysReg>,
    /// Caller-saved registers assigned to call-spanning values (Phase 2b).
    /// Maps PhysReg ID → list of (interval_start, interval_end) for each value
    /// assigned to that register. Used for selective save/restore at call sites.
    pub caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>>,
    /// The liveness analysis computed during register allocation, if any.
    /// Cached here so that calculate_stack_space_common can reuse it for
    /// Tier 2 liveness-based stack slot packing, avoiding a redundant
    /// O(blocks * values * iterations) dataflow computation.
    /// None when no registers were available (empty available_regs).
    pub liveness: Option<super::liveness::LivenessResult>,
}

/// Configuration for the register allocator.
pub struct RegAllocConfig {
    /// Available callee-saved registers for allocation (e.g., s1-s11 for RISC-V).
    pub available_regs: Vec<PhysReg>,
    /// Available caller-saved registers for allocation.
    /// These are assigned to values whose live ranges do NOT span any call.
    /// Since they don't cross calls, no prologue/epilogue save/restore is needed.
    /// Examples: x86 r11, r10, r8, r9.
    pub caller_saved_regs: Vec<PhysReg>,
    /// Whether to allow inline asm operands to be register-allocated.
    /// Only enable this when the backend's asm emitter checks reg_assignments
    /// before falling back to stack access. Currently only RISC-V does this.
    pub allow_inline_asm_regalloc: bool,
    /// Available XMM registers for F64 allocation (caller-saved, non-call-spanning).
    /// Examples: x86 xmm2-xmm7 (PhysReg 20-25).
    pub xmm_regs: Vec<PhysReg>,
}

/// Filter live intervals to only those eligible for register allocation,
/// using the same whitelist + ineligibility rules as the three-phase allocator.
fn filter_eligible_intervals(
    liveness: &LivenessResult,
    eligible: &FxHashSet<u32>,
) -> Vec<LiveInterval> {
    liveness
        .intervals
        .iter()
        .filter(|iv| eligible.contains(&iv.value_id))
        .filter(|iv| iv.end > iv.start)
        .copied()
        .collect()
}

/// ms178: Build Copy-coalescing groups.
///
/// Union-find over `Copy { dest, src: Value(v) }` instructions where both ends
/// are eligible for register allocation. A group is only kept when its members'
/// live intervals are pairwise DISJOINT — only then can a single register
/// legally hold all members at distinct times (the copy becomes a no-op).
/// Members that overlap another member of their group are split out.
///
/// Returns map: group leader value id -> all member value ids (incl. leader).
fn build_coalesce_groups(
    func: &IrFunction,
    liveness: &LivenessResult,
    eligible: &FxHashSet<u32>,
) -> FxHashMap<u32, Vec<u32>> {
    use crate::common::fx_hash::FxHashMap;
    if std::env::var("CCC_DEBUG_COALESCE").is_ok() {
        eprintln!("[COALESCE] fn={} build_coalesce_groups ENTERED", func.name);
    }
    // interval lookup
    let iv_of = |vid: u32| -> Option<(u32, u32)> {
        liveness
            .intervals
            .iter()
            .find(|iv| iv.value_id == vid)
            .map(|iv| (iv.start, iv.end))
    };
    // union-find
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    fn find(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while parent.get(&r).copied() != Some(r) {
            r = parent.get(&r).copied().unwrap_or(r);
        }
        // path compression
        let mut c = x;
        while parent.get(&c).copied().map(|p| p != r).unwrap_or(false) {
            let next = parent.get(&c).copied().unwrap_or(c);
            parent.insert(c, r);
            c = next;
        }
        r
    }
    fn union(parent: &mut FxHashMap<u32, u32>, a: u32, b: u32) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent.insert(rb, ra);
        }
    }
    let overlaps = |a: (u32, u32), b: (u32, u32)| a.0 < b.1 && b.0 < a.1;

    // Pass 1: union copy pairs.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } = inst
            {
                let d = dest.0;
                let s = v.0;
                if eligible.contains(&d) && eligible.contains(&s) {
                    parent.entry(d).or_insert(d);
                    parent.entry(s).or_insert(s);
                    union(&mut parent, d, s);
                }
            }
        }
    }
    if parent.is_empty() {
        if std::env::var("CCC_DEBUG_COALESCE").is_ok() {
            let n_copies = func
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter(|i| matches!(i, Instruction::Copy { .. }))
                .count();
            eprintln!(
                "[COALESCE] fn={} no eligible copy pairs (copies={})",
                func.name, n_copies
            );
        }
        return FxHashMap::default();
    }
    // Debug: show eligible copies whose endpoints' intervals OVERLAP (the
    // pairs coalescing cannot merge) — points at conservative liveness.
    if std::env::var("CCC_DEBUG_COALESCE").is_ok() {
        let mut n_rej = 0usize;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src: Operand::Value(v) } = inst {
                    let d = dest.0; let sv = v.0;
                    if eligible.contains(&d) && eligible.contains(&sv) {
                        if let (Some(di), Some(si)) = (iv_of(d), iv_of(sv)) {
                            if overlaps(di, si) {
                                if n_rej < 12 {
                                    eprintln!("[COALESCE] {} overlap copy v{}[{}-{}] <- v{}[{}-{}]",
                                        func.name, d, di.0, di.1, sv, si.0, si.1);
                                }
                                n_rej += 1;
                            }
                        }
                    }
                }
            }
        }
        eprintln!("[COALESCE] {} overlapping eligible copies: {}", func.name, n_rej);
    }

    // Pass 2: group members by leader.
    let mut groups: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let vids: Vec<u32> = parent.keys().copied().collect();
    for vid in vids {
        let leader = find(&mut parent, vid);
        groups.entry(leader).or_default().push(vid);
    }
    // Pass 3: split overlapping members out of their groups.
    let mut result: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (leader, members) in groups {
        // sort by start for the greedy disjoint check
        let mut sorted = members.clone();
        sorted.sort_by_key(|&m| iv_of(m).map(|x| x.0).unwrap_or(0));
        let mut accepted: Vec<u32> = Vec::new();
        let mut rejected: Vec<u32> = Vec::new();
        for m in sorted {
            let mi = iv_of(m);
            let mut ok = true;
            if let Some(mi) = mi {
                for &a in &accepted {
                    if let Some(ai) = iv_of(a) {
                        if overlaps(mi, ai) {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            if ok {
                accepted.push(m);
            } else {
                rejected.push(m);
            }
        }
        if accepted.len() > 1 {
            // The group representative is the union-find leader IF it is among
            // the accepted (pairwise-disjoint) members; otherwise use the
            // earliest-start accepted member. CRITICAL: the representative's
            // value id must be a member whose interval is covered by the merged
            // [min,max] interval — a rejected leader whose original interval
            // extends beyond the merged range would otherwise keep uses outside
            // the merged interval with no register coverage (miscompile).
            let rep = if accepted.contains(&leader) {
                leader
            } else {
                accepted[0]
            };
            result.insert(rep, accepted);
        }
        // rejected members keep their own intervals (not coalesced)
        let _ = rejected;
    }
    if std::env::var("CCC_DEBUG_COALESCE").is_ok() {
        let mut n_merged = 0usize;
        let mut n_members = 0usize;
        for (_, members) in &result {
            n_merged += 1;
            n_members += members.len();
        }
        eprintln!(
            "[COALESCE] fn={} groups={} members={}",
            func.name, n_merged, n_members
        );
        for (leader, members) in &result {
            let ivs: Vec<String> = members
                .iter()
                .map(|m| {
                    liveness
                        .intervals
                        .iter()
                        .find(|x| x.value_id == *m)
                        .map(|x| format!("val{}[{}-{}]", x.value_id, x.start, x.end))
                        .unwrap_or_else(|| format!("val{}[?]", m))
                })
                .collect();
            eprintln!("[COALESCE]   group leader={} members={:?}", leader, ivs);
        }
    }
    result
}

/// Run the register allocator on a function.
///
/// Strategy: We assign callee-saved registers to values with the longest
/// live intervals. This is a simplified linear scan that doesn't split
/// intervals — values either get a register for their entire lifetime or
/// remain on the stack.
///
/// We avoid allocating registers to:
/// - Alloca values (they represent stack addresses)
/// - i128/float values (they need special register paths)
/// - Values used only once right after definition (no benefit from register)
pub fn allocate_registers(func: &IrFunction, config: &RegAllocConfig) -> RegAllocResult {
    if config.available_regs.is_empty() && config.caller_saved_regs.is_empty() {
        return RegAllocResult {
            assignments: FxHashMap::default(),
            used_regs: Vec::new(),
            caller_save_spans: FxHashMap::default(),
            liveness: None,
        };
    }

    // Note: Register allocation is now enabled for functions with atomics.
    // Atomic operations in all backends (x86, ARM, RISC-V) access their operands
    // exclusively through regalloc-aware helpers (operand_to_rax/x0/t0 and
    // store_rax_to/x0_to/t0_to), so register-allocated values work correctly.
    // The atomic pointer operands are individually excluded from register
    // allocation eligibility below since they need stable stack addresses
    // for the memory access instructions.

    // On 32-bit targets, I64/U64 values need two registers (eax:edx) and cannot
    // be allocated to a single callee-saved register. Exclude them from eligibility.
    let is_32bit = crate::common::types::target_is_32bit();

    // Liveness analysis now uses backward dataflow iteration to correctly
    // handle loops (values live across back-edges have their intervals extended).
    let liveness = compute_live_intervals(func);

    // Count uses per value for prioritization, weighted by loop depth.
    //
    // Uses inside loops are weighted more heavily because they execute more
    // frequently. A use inside a loop at depth D contributes 10^D to the
    // weighted use count (so a use in a singly-nested loop counts 10x, doubly-
    // nested counts 100x, etc.). This ensures inner-loop temporaries get
    // priority for register allocation over values in straight-line code,
    // which is critical for performance in compute-heavy loops like zlib's
    // deflate_slow, longest_match, and slide_hash.
    let mut use_count: FxHashMap<u32, u64> = FxHashMap::default();

    // Precompute per-block loop weight: 10^depth, capped to avoid overflow.
    let block_loop_weight: Vec<u64> = liveness
        .block_loop_depth
        .iter()
        .map(|&d| {
            match d {
                0 => 1,
                1 => 10,
                2 => 100,
                3 => 1000,
                _ => 10_000, // cap at 10K for very deep nesting
            }
        })
        .collect();

    // Collect values whose types don't fit in a single GPR.
    let non_gpr_values = collect_non_gpr_values(func, is_32bit);

    // Helper closure to check if a type is unsuitable for GPR allocation
    let is_non_gpr_type = |ty: &IrType| -> bool {
        ty.is_float()
            || ty.is_long_double()
            || matches!(ty, IrType::I128 | IrType::U128)
            || (is_32bit && matches!(ty, IrType::I64 | IrType::U64))
    };

    // Use a whitelist approach: only allocate registers for values produced
    // by simple, well-understood instructions that store results via the
    // standard accumulator path (e.g., store_rax_to on x86, store_t0_to on RISC-V).
    let mut eligible: FxHashSet<u32> = FxHashSet::default();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        // Get the loop weight for this block (default 1 if no loop info available).
        let weight: u64 = if block_idx < block_loop_weight.len() {
            block_loop_weight[block_idx]
        } else {
            1
        };

        for inst in &block.instructions {
            // Values eligible for register allocation: those stored via the
            // standard accumulator path (store_rax_to on x86, store_t0_to on RISC-V).
            // Exclude float and i128 types since they use different register paths.
            match inst {
                Instruction::BinOp { dest, ty, .. } | Instruction::UnaryOp { dest, ty, .. } => {
                    if !is_non_gpr_type(ty) {
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
                    if !is_non_gpr_type(to_ty) && !is_non_gpr_type(from_ty) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    if !is_non_gpr_type(ty) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::GetElementPtr { dest, .. } => {
                    eligible.insert(dest.0);
                }
                Instruction::Copy { dest, src: _ } => {
                    // Copy instructions are eligible unless the source produces a
                    // non-GPR value (float, i128, or i64 on 32-bit). We check both
                    // constant types and propagated non-GPR status from Value sources.
                    if !non_gpr_values.contains(&dest.0) {
                        eligible.insert(dest.0);
                    }
                }
                // Call results are eligible for callee-saved register allocation.
                // The result arrives in the accumulator (rax on x86, x0 on ARM, a0 on
                // RISC-V), and emit_call_store_result calls emit_store_result which
                // uses store_rax_to/store_t0_to — both of which are register-aware
                // and will emit a reg-to-reg move (e.g., movq %rax, %rbx) instead of
                // a stack spill.
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        if !is_non_gpr_type(&info.return_type) {
                            eligible.insert(dest.0);
                        }
                    }
                }
                Instruction::Select { dest, ty, .. } => {
                    if !is_non_gpr_type(ty) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::GlobalAddr { dest, .. } | Instruction::LabelAddr { dest, .. } => {
                    eligible.insert(dest.0);
                }
                // Atomic operations store their results via store_rax_to/store_t0_to.
                Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. } => {
                    if !is_non_gpr_type(ty) {
                        eligible.insert(dest.0);
                    }
                }
                Instruction::ParamRef { dest, ty, .. } => {
                    if !is_non_gpr_type(ty) {
                        eligible.insert(dest.0);
                    }
                }
                _ => {}
            }

            // Count uses of operands, weighted by loop depth of the containing block.
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += weight;
                }
            });
            // W1: address-side uses (Load/Store ptr, GEP base) carry the same
            // loop-weighted heat — hot addressing bases must rank high in the
            // allocation/spill-order decisions driven by this count.
            for_each_value_use_in_instruction(inst, |v| {
                *use_count.entry(v.0).or_insert(0) += weight;
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += weight;
            }
        });
    }

    // Exclude values used as pointers in instructions whose codegen paths use
    // resolve_slot_addr() directly (not register-aware).
    remove_ineligible_operands(func, &mut eligible, config);

    // --- 3-channel multiply ILP ---
    //
    // For loops with many multiply-accumulate patterns (a += b*c), we want 3
    // independent multiply chains to fully utilize the CPU's multiply port
    // (which has 3-cycle latency but 1-cycle throughput). The linear scan
    // naturally provides 2 temp registers via rotation. By excluding every
    // 3rd fusible multiply temp from allocation, it falls through to the
    // accumulator path (%eax) in the codegen, creating a 3rd channel.
    //
    // Pattern: r12, rbx, %eax, r12, rbx, %eax, ...
    exclude_every_third_mul_temp(func, &mut eligible);

    // --- Phi register coalescing ---
    //
    // For loop-carried phi variables, the backedge source value (the new value
    // computed in the loop body) should share the same register as the phi dest
    // (the value at the loop header). This eliminates the register-to-register
    // or register-to-stack copy at the backedge.
    //
    // We detect backedge Copy instructions where the dest is a multi-def value
    // (phi dest after phi elimination) and the source is a loop-local value.
    // The backedge source is removed from the eligible set so it doesn't get
    // allocated independently. After allocation, it inherits the phi dest's
    // register assignment.
    let phi_coalesce = if std::env::var("CCC_NO_PHI_COALESCE").is_ok() {
        Vec::new()
    } else {
        detect_phi_coalesce_groups(func, &liveness)
    };
    for &(_phi_dest, backedge_src) in &phi_coalesce {
        // Remove backedge source from eligibility — it will inherit the phi dest's register.
        eligible.remove(&backedge_src);
    }

    // --- Linear scan allocation (replaces three-phase greedy allocator) ---
    //
    // Phase 1: callee-saved registers for ALL eligible values.
    //   Callee-saved regs are safe across calls, so they can hold any value.
    //   Linear scan gives better coverage than the old greedy approach by
    //   considering interval overlap rather than just "does it span a call".
    //
    // Phase 2: caller-saved registers for eligible, non-call-spanning values
    //   that weren't allocated in Phase 1. Caller-saved regs are destroyed by
    //   calls so they can only hold values that don't cross call boundaries.

    let call_points = &liveness.call_points;

    // ms178: pre-allocation COPY COALESCING.
    //
    // Linear scan allocates registers per SSA value; Copy instructions whose
    // dest/src are disjoint should share a register so the copy disappears.
    // The existing hint mechanism only works when the copy DEST finds a free
    // register at its (late, in-loop) start point — under register pressure it
    // never does, so every loop-carried phi copy-back round-trips through a
    // stack slot (gzip's longest_match: ~922 spill refs in the hot loop).
    //
    // Here we union Copy-related values into a single coalesced interval
    // BEFORE allocation (dest's range extends back to src's start), so the
    // coalesced interval is allocated as one unit with the src's early start.
    // Soundness: members are only merged when their live intervals are pairwise
    // disjoint (then one register provably holds all of them at distinct
    // times). Any member that overlaps another in its group is split out and
    // keeps its own interval. The final assignments map then maps every member
    // to the group leader's register, making the copies no-ops in codegen.
    // Opt-in (CCC_COALESCE=1): measured neutral on gzip (loop-carried values
    // genuinely overlap, so only ~2 groups coalesce in longest_match), sound
    // after the representative fix, and a net win on copy-heavy non-loop code.
    // Kept OFF by default so the default codegen matches the fully-validated
    // v14 baseline.
    let coalesce_groups: FxHashMap<u32, Vec<u32>> = if std::env::var("CCC_COALESCE").is_ok() {
        build_coalesce_groups(func, &liveness, &eligible)
    } else {
        FxHashMap::default()
    };

    // Phase 1: callee-saved registers only for call-spanning values (they pay
    // a whole-function push/pop; non-spanning values prefer the free
    // caller-saved pool in Phase 2, with callee-saved spillover in Phase 2c).
    let mut phase1_intervals: Vec<LiveInterval> = liveness
        .intervals
        .iter()
        .filter(|iv| eligible.contains(&iv.value_id))
        .filter(|iv| iv.end > iv.start)
        .filter(|iv| spans_any_call(iv, call_points))
        .copied()
        .collect();
    // Replace each coalesced group's member intervals with ONE merged interval
    // whose id is the group leader, and record the member→leader map for
    // assignment propagation.
    let mut coalesce_member_of: FxHashMap<u32, u32> = FxHashMap::default();
    if !coalesce_groups.is_empty() {
        let mut removed: FxHashSet<u32> = FxHashSet::default();
        for (leader, members) in &coalesce_groups {
            let mut start = u32::MAX;
            let mut end = 0u32;
            for m in members {
                removed.insert(*m);
                if let Some(iv) = liveness.intervals.iter().find(|x| x.value_id == *m) {
                    if iv.start < start {
                        start = iv.start;
                    }
                    if iv.end > end {
                        end = iv.end;
                    }
                }
            }
            for m in members {
                if *m != *leader {
                    coalesce_member_of.insert(*m, *leader);
                }
            }
            phase1_intervals.push(LiveInterval {
                value_id: *leader,
                start,
                end,
            });
        }
        phase1_intervals.retain(|iv| !removed.contains(&iv.value_id));
    }
    let mut phase1_ranges =
        live_range::build_live_ranges(&phase1_intervals, &liveness.block_loop_depth, func);
    // Bump coalesced leaders' priority by group size so merged intervals with
    // many uses (sum of member uses) aren't underweighted vs their single
    // interval peers.
    if !coalesce_groups.is_empty() {
        for r in phase1_ranges.iter_mut() {
            if let Some(members) = coalesce_groups.get(&r.value_id) {
                let n = members.len().max(1) as u64;
                r.priority = r.priority.saturating_mul(n.min(8));
                r.calculate_spill_weight();
            }
        }
    }
    let mut allocator = LinearScanAllocator::new(phase1_ranges, config.available_regs.clone());
    allocator.run();

    let mut assignments = allocator.assignments;
    // Propagate coalesced registers to all group members.
    if !coalesce_member_of.is_empty() {
        for (member, leader) in &coalesce_member_of {
            if let Some(&reg) = assignments.get(leader) {
                assignments.insert(*member, reg);
            }
        }
    }
    // `used_regs_set` is deliberately the ABI callee-saved set. It feeds the
    // x86 prologue/epilogue save list, so caller-saved allocations must never
    // enter it. Keep a separate exclusion set for Phase 2b instead.
    let mut used_regs_set: FxHashSet<u8> = FxHashSet::default();
    for &reg in assignments.values() {
        used_regs_set.insert(reg.0);
    }
    let mut caller_used_regs_set: FxHashSet<u8> = FxHashSet::default();

    // Phase 2: caller-saved linear scan for unallocated non-call-spanning values.
    if !config.caller_saved_regs.is_empty() {
        let phase2_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| eligible.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !spans_any_call(iv, call_points))
            .copied()
            .collect();

        if !phase2_intervals.is_empty() {
            let phase2_ranges =
                live_range::build_live_ranges(&phase2_intervals, &liveness.block_loop_depth, func);
            let mut caller_allocator =
                LinearScanAllocator::new(phase2_ranges, config.caller_saved_regs.clone());
            caller_allocator.run();

            for (vid, reg) in caller_allocator.assignments {
                assignments.insert(vid, reg);
                // Phase 2 values are proven not to span calls; their ABI
                // caller-saved registers need neither entry save nor exit
                // restore. The old insertion into used_regs_set caused every
                // such register to be pushed/popped as if callee-saved.
                caller_used_regs_set.insert(reg.0);
            }
        }
    }

    // Phase 2c: give call-free hot-loop overflow the unused callee-saved
    // registers instead of stack slots (one push/pop amortizes over the loop;
    // a spill costs a store+load per iteration). Runs after Phase 2 so the
    // free caller-saved pool is always preferred.
    {
        let phase2c_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| eligible.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !spans_any_call(iv, call_points))
            .copied()
            .collect();
        if !phase2c_intervals.is_empty() {
            let free_callee: Vec<PhysReg> = config
                .available_regs
                .iter()
                .filter(|r| !used_regs_set.contains(&r.0))
                .copied()
                .collect();
            if !free_callee.is_empty() {
                let phase2c_ranges =
                    live_range::build_live_ranges(&phase2c_intervals, &liveness.block_loop_depth, func);
                let mut spill_allocator = LinearScanAllocator::new(phase2c_ranges, free_callee);
                // NOTE: future-value exchange eviction was measured here too
                // (2026-08-10) and ALSO regressed gzip (+5.75%/-6, +5.36%/-9;
                // longest_match 304->339 insns, 92->128 rsp refs): hot chains
                // evicting each other cancels out and second-order effects lose.
                // Mode-3's next-use guard remains the best-measured policy.
                spill_allocator.run();
                for (vid, reg) in spill_allocator.assignments {
                    assignments.insert(vid, reg);
                    // These ARE callee-saved: the prologue/epilogue must
                    // save/restore them.
                    used_regs_set.insert(reg.0);
                }
            }
        }
    }

    // Debug: count overlaps BEFORE phi coalesce
    if std::env::var("CCC_VERIFY_REGALLOC").is_ok() {
        let mut pre_count = 0;
        let mut pre_reg_ivs: crate::common::fx_hash::FxHashMap<u8, Vec<(u32, u32, u32)>> =
            crate::common::fx_hash::FxHashMap::default();
        for iv in &liveness.intervals {
            if let Some(&reg) = assignments.get(&iv.value_id) {
                pre_reg_ivs
                    .entry(reg.0)
                    .or_default()
                    .push((iv.start, iv.end, iv.value_id));
            }
        }
        for (_, intervals) in &pre_reg_ivs {
            for i in 0..intervals.len() {
                for j in (i + 1)..intervals.len() {
                    let (s1, e1, _) = intervals[i];
                    let (s2, e2, _) = intervals[j];
                    if s1 < e2 && s2 < e1 {
                        pre_count += 1;
                    }
                }
            }
        }
        if pre_count > 0 {
            eprintln!(
                "[REGALLOC-PRE-PHI] {} overlaps BEFORE phi coalesce",
                pre_count
            );
        }
    }

    // Propagate phi coalesce assignments: backedge source values inherit
    // the register of their phi dest. This makes the backedge Copy a no-op
    // when both values share the same register.
    // Safety check: only propagate if the backedge source's interval doesn't
    // conflict with other values already assigned to the same register.
    for &(phi_dest, backedge_src) in &phi_coalesce {
        if let Some(&reg) = assignments.get(&phi_dest) {
            // Find backedge_src's interval
            let src_interval = liveness
                .intervals
                .iter()
                .find(|iv| iv.value_id == backedge_src);
            if let Some(src_iv) = src_interval {
                // Additional: check overlap with the phi dest's own interval
                // (they share a register, so they should not overlap)
                let dest_iv = liveness.intervals.iter().find(|iv| iv.value_id == phi_dest);
                if let Some(_div) = dest_iv {
                    // The coarse interval-overlap test rejects the common
                    // loop-carried pattern (head/condition use of the phi dest
                    // precedes the backedge source's definition, so sharing a
                    // register is safe). Only a real "lost copy" is a hazard:
                    // the OLD phi-dest value used AFTER the new value is
                    // computed and BEFORE the backedge copy. That window lies
                    // in the copy block and the source's defining block, which
                    // the group detector already checks; re-verify here.
                    let phi_dest_used_after_src = {
                        let src_def_block = func.blocks.iter().enumerate().find_map(
                            |(bi, b)| {
                                b.instructions
                                    .iter()
                                    .any(|i| i.dest().is_some_and(|d| d.0 == backedge_src))
                                    .then_some(bi)
                            },
                        );
                        let copy_block = func.blocks.iter().enumerate().find_map(|(bi, b)| {
                            b.instructions.iter().any(|i| {
                                matches!(i, Instruction::Copy { dest, src: Operand::Value(sv) }
                                    if dest.0 == phi_dest && sv.0 == backedge_src)
                            })
                            .then_some(bi)
                        });
                        let mut hazard = false;
                        for bi in [src_def_block, copy_block].into_iter().flatten() {
                            let mut src_defined = false;
                            for inst in &func.blocks[bi].instructions {
                                if !src_defined {
                                    if inst.dest().is_some_and(|d| d.0 == backedge_src) {
                                        src_defined = true;
                                    }
                                } else if uses_value(inst, phi_dest) {
                                    hazard = true;
                                }
                            }
                        }
                        hazard
                    };
                    if phi_dest_used_after_src {
                        // Genuine lost-copy hazard — keep them apart.
                        continue;
                    }
                }
                // Check for conflicts with other values in the same register
                let has_conflict = liveness.intervals.iter().any(|iv| {
                    if iv.value_id == backedge_src || iv.value_id == phi_dest {
                        return false;
                    }
                    if let Some(&other_reg) = assignments.get(&iv.value_id) {
                        other_reg.0 == reg.0 && iv.start < src_iv.end && src_iv.start < iv.end
                    } else {
                        false
                    }
                });
                if !has_conflict {
                    assignments.insert(backedge_src, reg);
                }
            } else {
                // No interval info — still safe to propagate (value might be dead)
                assignments.insert(backedge_src, reg);
            }
        }
    }

    // Debug: count overlaps after phi coalesce
    if std::env::var("CCC_VERIFY_REGALLOC").is_ok() {
        let mut overlap_count = 0;
        let mut reg_ivs: crate::common::fx_hash::FxHashMap<u8, Vec<(u32, u32, u32)>> =
            crate::common::fx_hash::FxHashMap::default();
        for iv in &liveness.intervals {
            if let Some(&reg) = assignments.get(&iv.value_id) {
                reg_ivs
                    .entry(reg.0)
                    .or_default()
                    .push((iv.start, iv.end, iv.value_id));
            }
        }
        for (_, intervals) in &reg_ivs {
            for i in 0..intervals.len() {
                for j in (i + 1)..intervals.len() {
                    let (s1, e1, _) = intervals[i];
                    let (s2, e2, _) = intervals[j];
                    if s1 < e2 && s2 < e1 {
                        overlap_count += 1;
                    }
                }
            }
        }
        if overlap_count > 0 {
            eprintln!(
                "[REGALLOC-POST-PHI] {} overlaps after phi coalesce",
                overlap_count
            );
        }
    }

    // Phase 3: XMM register allocation for F64 values that don't span calls.
    // These values were excluded from GPR allocation but can use XMM registers.
    if !config.xmm_regs.is_empty() {
        // Collect F64 values: values in non_gpr_values that are F64 typed,
        // haven't been assigned a GPR, and don't span calls.
        let f64_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| non_gpr_values.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !spans_any_call(iv, call_points))
            // Only include values that are actually F64 (not i128, not f32, etc.)
            .filter(|iv| {
                // Check if this value is produced by a F64-typed instruction
                func.blocks.iter().any(|block| {
                    block.instructions.iter().any(|inst| match inst {
                        Instruction::BinOp { dest, ty, .. }
                        | Instruction::UnaryOp { dest, ty, .. }
                            if *ty == IrType::F64 =>
                        {
                            dest.0 == iv.value_id
                        }
                        Instruction::Load { dest, ty, .. } if *ty == IrType::F64 => {
                            dest.0 == iv.value_id
                        }
                        Instruction::Cast { dest, to_ty, .. } if *to_ty == IrType::F64 => {
                            dest.0 == iv.value_id
                        }
                        _ => false,
                    })
                })
            })
            .copied()
            .collect();

        if !f64_intervals.is_empty() {
            let f64_ranges =
                live_range::build_live_ranges(&f64_intervals, &liveness.block_loop_depth, func);
            let mut xmm_allocator = LinearScanAllocator::new(f64_ranges, config.xmm_regs.clone());
            xmm_allocator.run();

            for (vid, reg) in xmm_allocator.assignments {
                assignments.insert(vid, reg);
                // XMM regs (20+) are caller-saved, no prologue save needed
            }
        }

        // Phase 3b (W6): XMM allocation for 128-bit VECTOR values. Runs AFTER
        // the F64 scan so it sees those assignments and avoids them. Pool =
        // xmm3-xmm7 (PhysReg 21-25): xmm0/xmm1 are accumulator/scratch and
        // xmm2 is an implicit pblendvb/VNNI/F128 scratch. Candidates are
        // strictly guarded by collect_vecreg_candidates; widening this set
        // without matching every consumer is unsound. Enabled after regression,
        // differential-CFG fuzz, gzip, and expat validation; retain an absolute
        // diagnostic kill switch for reproducibility.
        let vecreg_enabled = std::env::var("CCC_NO_VECREG").is_err();
        if vecreg_enabled {
            let vec_candidates = collect_vecreg_candidates(func);
            if !vec_candidates.is_empty() {
                let mut vec_pool: Vec<PhysReg> = (21..=25).map(PhysReg).collect();
                vec_pool.retain(|r| !assignments.values().any(|a| a.0 == r.0));
                if !vec_pool.is_empty() {
                    // Alloca values have no liveness intervals (liveness is
                    // computed for non-alloca values only). Build synthetic
                    // intervals with the SAME program-point convention
                    // (per-instruction +1, per-terminator +1) so the
                    // call-span check and the linear scan are consistent.
                    let vec_intervals = synthetic_vec_intervals(func, &vec_candidates);
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

    // Phase 2b: Caller-saved registers for call-spanning values with
    // per-call selective save/restore. Unlike Phase 2 (non-call-spanning only),
    // Phase 2b allows call-spanning values in caller-saved registers by
    // recording their live intervals. The codegen saves/restores each register
    // only at call sites where the value is actually live.
    let mut caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
    if !config.caller_saved_regs.is_empty() && std::env::var("CCC_CALLER_SAVE_SPANNING").is_ok() {
        // Use all available caller-saved registers (not just r10/r11)
        let span_regs: Vec<PhysReg> = config
            .caller_saved_regs
            .iter()
            // Do not independently allocate a Phase 2b call-spanning value
            // to a register already owned by the Phase 2 non-spanning scan.
            // This is an allocation-conflict guard, not a prologue-save rule.
            .filter(|r| !caller_used_regs_set.contains(&r.0))
            .copied()
            .collect();

        if !span_regs.is_empty() {
            let phase2b_intervals: Vec<LiveInterval> = liveness
                .intervals
                .iter()
                .filter(|iv| eligible.contains(&iv.value_id))
                .filter(|iv| iv.end > iv.start)
                .filter(|iv| !assignments.contains_key(&iv.value_id))
                .filter(|iv| spans_any_call(iv, call_points))
                .take(500) // Limit to top 500 candidates to avoid O(n²) in linear scan
                .copied()
                .collect();

            if !phase2b_intervals.is_empty() {
                // Build interval map for live-at-call checks
                let interval_map: FxHashMap<u32, (u32, u32)> = phase2b_intervals
                    .iter()
                    .map(|iv| (iv.value_id, (iv.start, iv.end)))
                    .collect();

                // Lightweight range builder — O(n) instead of O(n × instructions).
                // The full build_live_ranges scans ALL function instructions for
                // use-site data, which is too slow for 4000+ Phase 2b intervals
                // in large functions like sqlite3VdbeExec.
                let mut phase2b_ranges: Vec<live_range::LiveRange> = phase2b_intervals
                    .iter()
                    .map(|iv| {
                        let mut r = live_range::LiveRange::from_interval(*iv, 0);
                        // Priority: inverse range length (shorter ranges = higher priority)
                        let len = (iv.end - iv.start).max(1) as u64;
                        r.priority = 1_000_000 / len;
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
                    // Record the interval for this register for selective save/restore
                    if let Some(&(start, end)) = interval_map.get(&vid) {
                        caller_save_spans
                            .entry(reg.0)
                            .or_default()
                            .push((start, end));
                    }
                    // Do NOT add to used_regs_set (not prologue-saved)
                }
            }
        }
    }

    let mut used_regs: Vec<PhysReg> = used_regs_set.iter().map(|&r| PhysReg(r)).collect();
    used_regs.sort_by_key(|r| r.0);

    // Diagnostic: dump eligible vs assigned per value, gated by CCC_DUMP_IR_FUNC.
    if let Ok(want) = std::env::var("CCC_DUMP_IR_FUNC") {
        if func.name.contains(&want) {
            let mut ev: Vec<_> = eligible.iter().copied().collect();
            ev.sort();
            let mut n_assigned = 0;
            for vid in &ev {
                let assigned = assignments
                    .get(vid)
                    .map(|r| r.0.to_string())
                    .unwrap_or_else(|| "-".into());
                let iv = liveness.intervals.iter().find(|x| x.value_id == *vid);
                let (s, e) = iv.map(|x| (x.start, x.end)).unwrap_or((0, 0));
                let pri = use_count.get(vid).copied().unwrap_or(0);
                if assignments.contains_key(vid) {
                    n_assigned += 1;
                }
                eprintln!(
                    "[ELIG] val{} [{}..{}] pri={} -> reg={}",
                    vid, s, e, pri, assigned
                );
            }
            eprintln!(
                "[ELIG-SUM] func={} eligible={} assigned={} regs={:?}",
                func.name,
                ev.len(),
                n_assigned,
                used_regs.iter().map(|r| r.0).collect::<Vec<_>>()
            );
        }
    }

    // Verify: no two assigned values should have overlapping live intervals
    // in the same physical register.
    if std::env::var("CCC_VERIFY_REGALLOC").is_ok() {
        let mut reg_intervals: crate::common::fx_hash::FxHashMap<u8, Vec<(u32, u32, u32)>> =
            crate::common::fx_hash::FxHashMap::default();
        for iv in &liveness.intervals {
            if let Some(&reg) = assignments.get(&iv.value_id) {
                reg_intervals
                    .entry(reg.0)
                    .or_default()
                    .push((iv.start, iv.end, iv.value_id));
            }
        }
        for (&reg_id, intervals) in &reg_intervals {
            for i in 0..intervals.len() {
                for j in (i + 1)..intervals.len() {
                    let (s1, e1, v1) = intervals[i];
                    let (s2, e2, v2) = intervals[j];
                    if s1 < e2 && s2 < e1 {
                        eprintln!(
                            "[REGALLOC-OVERLAP] reg={} val{}[{}-{}] overlaps val{}[{}-{}]",
                            reg_id, v1, s1, e1, v2, s2, e2
                        );
                    }
                }
            }
        }
    }

    if std::env::var("CCC_DEBUG_REGALLOC").is_ok() && eligible.len() > 50 {
        let total_eligible = eligible.len();
        let total_assigned = assignments.len();
        let total_intervals = liveness.intervals.len();
        let non_call_spanning = liveness
            .intervals
            .iter()
            .filter(|iv| {
                eligible.contains(&iv.value_id)
                    && !spans_any_call(iv, call_points)
                    && iv.end > iv.start
            })
            .count();
        let call_spanning = liveness
            .intervals
            .iter()
            .filter(|iv| {
                eligible.contains(&iv.value_id)
                    && spans_any_call(iv, call_points)
                    && iv.end > iv.start
            })
            .count();
        eprintln!("[REGALLOC] {} eligible, {} assigned ({:.0}%), {} call-spanning, {} non-call, {} callee, {} caller",
            total_eligible, total_assigned,
            if total_eligible > 0 { total_assigned as f64 / total_eligible as f64 * 100.0 } else { 0.0 },
            call_spanning, non_call_spanning,
            config.available_regs.len(), config.caller_saved_regs.len());
    }

    RegAllocResult {
        assignments,
        used_regs,
        caller_save_spans,
        liveness: Some(liveness),
    }
}

/// v5 CCC_ENABLE_VECREG: collect the 128-bit VECTOR VALUES that are safe to
/// hold in an XMM register for their whole live range.
///
/// A vector value here is the ALLOCA that a SIMD intrinsic writes its 16-byte
/// result into (the `dest_ptr` of e.g. Pcmpeqb128 / Pxor128). The backend
/// redirects the intrinsic's `movdqu %xmm0, slot` into `movdqa %xmm0, %xmmN`
/// and every subsequent cache-aware vector load of the value into
/// `movdqa %xmmN, %xmm0`; live-out values are flushed to their slot at block
/// ends.
///
/// Candidate guards (each mirrors a codegen constraint — the "phase-3
/// candidate guards matching the non-GPR collector" fix; a candidate outside
/// this whitelist miscompiles, e.g. the fold_4 class):
/// 1. V is the dest_ptr of at least one 128-bit vector-producing intrinsic
///    (NOT a store-target op like Storedqu, whose dest_ptr is a user pointer).
/// 2. V is not volatile and not over-aligned beyond 16 (runtime-alignment
///    dance conflicts with the redirect).
/// 3. V has >= 2 intrinsic-ARG uses (single-use values are already handled by
///    the deferred-store optimization — allocating a register to them is the
///    "net-negative on cast-heavy/single-use code" regression).
/// 4. Every use of V is either an intrinsic dest_ptr (a write) or an
///    intrinsic ARG of a cache-aware consumer (sse_load_arg/avx_load_arg_to).
///    Excluded consumers read V through the scalar address path or through
///    direct slot reads (stale under the redirect): the FMA / load /
///    horizontal-reduction / auto-vectorizer family, plus the raw movq
///    load/store helpers.
/// 5. V is never used by a non-Intrinsic instruction (no scalar operand use,
///    no ptr/base use, no memcpy src/dest, no call arg) and never used by a
///    terminator.
/// 6. Non-call-spanning (XMM regs are caller-saved) — enforced by the caller.
fn collect_vecreg_candidates(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp;
    use crate::backend::liveness::{
        for_each_operand_in_instruction, for_each_value_use_in_instruction,
        for_each_operand_in_terminator,
    };
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

    let is_128_producer = |op: &IntrinsicOp| -> bool {
        use IntrinsicOp as O;
        matches!(
            op,
            O::Pcmpeqb128 | O::Pcmpeqd128 | O::Psubusb128 | O::Psubsb128
            | O::Por128 | O::Pand128 | O::Pxor128
            | O::AddPs128 | O::SubPs128 | O::MulPs128
            | O::AddPd128 | O::SubPd128 | O::MulPd128
            | O::Paddw128 | O::Psubw128 | O::Pmulhw128 | O::Pmullw128
            | O::Pmuludq128 | O::Pmuldq128 | O::Pmulld128
            | O::Pmaddwd128 | O::Pmaddubsw128
            | O::Pcmpgtw128 | O::Pcmpgtb128
            | O::Paddd128 | O::Psubd128
            | O::Paddb128 | O::Psubb128 | O::Psubusw128
            | O::Psadbw128
            | O::Pshufb128 | O::Pabsb128 | O::Pabsw128 | O::Pabsd128
            | O::Pmaxub128 | O::Pminub128
            | O::Pmovzxbw128 | O::Pmovzxwd128
            | O::Packssdw128 | O::Packsswb128 | O::Packuswb128
            | O::Punpcklbw128 | O::Punpckhbw128
            | O::Punpcklwd128 | O::Punpckhwd128
            | O::Phaddw128 | O::Phaddd128 | O::Palignr128
            | O::Psllw128 | O::Psrlw128
            | O::Pblendvb128
            | O::Aesenc128 | O::Aesenclast128 | O::Aesdec128 | O::Aesdeclast128
            | O::Aesimc128 | O::Aeskeygenassist128
            | O::Pclmulqdq128
            | O::Gf2p8mulb128 | O::Gf2p8affineqb128 | O::Gf2p8affineinvqb128
            | O::Psllwi128 | O::Psrlwi128 | O::Psrawi128 | O::Psradi128
            | O::Pslldi128 | O::Psrldi128 | O::Pslldqi128 | O::Psrldqi128
            | O::Psllqi128 | O::Psrlqi128
            | O::Pshufd128 | O::Pshuflw128 | O::Pshufhw128
            | O::Loaddqu | O::Loadldi128
            | O::SetEpi8 | O::SetEpi16 | O::SetEpi32 | O::Cvtsi32Si128
            | O::Cast256to128
            | O::AddF64x2 | O::MulF64x2 | O::AddI32x4
            | O::Dpbusd128 | O::Dpbusds128 | O::Dpwusd128 | O::Dpwusds128
            | O::Dpbssd128 | O::Dpbssds128 | O::Dpbsud128 | O::Dpbsuds128
            | O::Dpbuud128 | O::Dpbuuds128 | O::Dpwuud128 | O::Dpwuuds128
            | O::Dpwssd128 | O::Dpwssds128
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
            O::FmaF64x2 | O::FmaF64x4 | O::FmaF64x4Hoisted | O::FmaF64x4SIB
            | O::BroadcastLoadF64
            | O::LoadF64x2 | O::LoadF64x4 | O::LoadI32x4 | O::LoadI32x8
            | O::HorizontalAddF64x2 | O::HorizontalAddF64x4
            | O::HorizontalAddI32x4 | O::HorizontalAddI32x8
            | O::VecLoadF64x2 | O::VecLoadF64x4 | O::VecLoadI32x4 | O::VecLoadI32x8
            | O::VecAddF64x2 | O::VecAddF64x4 | O::VecAddI32x4 | O::VecAddI32x8
            | O::VecMulF64x2 | O::VecMulF64x4
            | O::VecHorizontalAddF64x2 | O::VecHorizontalAddF64x4
            | O::VecHorizontalAddI32x4 | O::VecHorizontalAddI32x8
        )
    };

    let mut produced: FxHashSet<u32> = FxHashSet::default();
    let mut store_target: FxHashSet<u32> = FxHashSet::default();
    let mut arg_uses: FxHashMap<u32, u32> = FxHashMap::default();
    let mut bad_use: FxHashSet<u32> = FxHashSet::default();
    let mut raw_consume: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Intrinsic {
                    dest_ptr: Some(d),
                    op,
                    args,
                    ..
                } => {
                    if is_128_producer(op) {
                        produced.insert(d.0);
                    }
                    if is_store_target(op) {
                        store_target.insert(d.0);
                    }
                    if is_raw_reader(op) {
                        for arg in args {
                            if let Operand::Value(v) = arg {
                                raw_consume.insert(v.0);
                            }
                        }
                    }
                    for arg in args {
                        if let Operand::Value(v) = arg {
                            *arg_uses.entry(v.0).or_insert(0) += 1;
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
        if store_target.contains(&v)
            || volatile_allocas.contains(&v)
            || over_align_allocas.contains(&v)
            || bad_use.contains(&v)
            || raw_consume.contains(&v)
        {
            continue;
        }
        if arg_uses.get(&v).copied().unwrap_or(0) < 2 {
            continue; // single-use: deferred-store handles it, register is waste
        }
        result.insert(v);
    }
    result
}

/// Build synthetic live intervals for the vecreg candidates (alloca values,
/// which the liveness analysis deliberately excludes). Uses the same
/// program-point convention as `liveness::assign_program_points`: every
/// instruction advances the point by 1, and every block terminator by 1, so
/// the resulting intervals are comparable with `call_points` and drive the
/// existing `LinearScanAllocator` unchanged.
///
/// Interval = [first intrinsic WRITE of V, last ARG use of V] in program
/// points. Read-modify-write patterns (e.g. a loop-carried `acc` that is both
/// the dest_ptr and an arg of the same intrinsic) naturally produce an
/// interval covering the loop.
fn synthetic_vec_intervals(
    func: &IrFunction,
    candidates: &FxHashSet<u32>,
) -> Vec<LiveInterval> {
    let mut first_write: FxHashMap<u32, u32> = FxHashMap::default();
    let mut last_use: FxHashMap<u32, u32> = FxHashMap::default();
    let mut point: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest_ptr: Some(d), ..
            } = inst
            {
                if candidates.contains(&d.0) {
                    first_write.entry(d.0).or_insert(point);
                }
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if candidates.contains(&v.0) {
                        last_use.insert(v.0, point);
                    }
                }
            });
            point += 1;
        }
        point += 1; // terminator point (matches assign_program_points)
    }
    let mut result = Vec::new();
    for &v in candidates {
        if let (Some(&s), Some(&e)) = (first_write.get(&v), last_use.get(&v)) {
            if e > s {
                result.push(LiveInterval {
                    value_id: v,
                    start: s,
                    end: e,
                });
            }
        }
    }
    result
}

/// Collect values whose types don't fit in a single GPR (floats, i128, and
/// on 32-bit targets: i64/u64). Copy instructions that chain from these
/// values must also be excluded via fixpoint propagation.
fn collect_non_gpr_values(func: &IrFunction, is_32bit: bool) -> FxHashSet<u32> {
    let is_non_gpr_type = |ty: &IrType| -> bool {
        ty.is_float()
            || ty.is_long_double()
            || matches!(ty, IrType::I128 | IrType::U128)
            || (is_32bit && matches!(ty, IrType::I64 | IrType::U64))
    };

    let mut non_gpr_values: FxHashSet<u32> = FxHashSet::default();

    // First pass: collect non-GPR values from typed instructions
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. } | Instruction::UnaryOp { dest, ty, .. } => {
                    if is_non_gpr_type(ty) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Cast {
                    dest,
                    to_ty,
                    from_ty,
                    ..
                } => {
                    if is_non_gpr_type(to_ty) || is_non_gpr_type(from_ty) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    if is_non_gpr_type(ty) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        if is_non_gpr_type(&info.return_type) {
                            non_gpr_values.insert(dest.0);
                        }
                    }
                }
                Instruction::Select { dest, ty, .. } => {
                    if is_non_gpr_type(ty) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. } => {
                    if is_non_gpr_type(ty) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Intrinsic {
                    dest: Some(d), op, ..
                } => {
                    // Vector intrinsics produce 128/256-bit values that cannot be
                    // stored in scalar GPRs. Exclude them from register allocation.
                    use crate::ir::intrinsics::IntrinsicOp;
                    let is_vector = matches!(
                        op,
                        IntrinsicOp::VecZeroF64x4
                            | IntrinsicOp::VecZeroF64x2
                            | IntrinsicOp::VecZeroI32x8
                            | IntrinsicOp::VecZeroI32x4
                            | IntrinsicOp::VecLoadF64x4
                            | IntrinsicOp::VecLoadF64x2
                            | IntrinsicOp::VecLoadI32x8
                            | IntrinsicOp::VecLoadI32x4
                            | IntrinsicOp::VecAddF64x4
                            | IntrinsicOp::VecAddF64x2
                            | IntrinsicOp::VecAddI32x8
                            | IntrinsicOp::VecAddI32x4
                            | IntrinsicOp::VecMulF64x4
                            | IntrinsicOp::VecMulF64x2
                    );
                    if is_vector {
                        non_gpr_values.insert(d.0);
                    }
                }
                _ => {}
            }
        }
    }

    // Propagate non-GPR status through Copy chains: if a Copy's source is a
    // non-GPR value, the dest is also non-GPR. Iterate until fixpoint since
    // Copies can chain (Copy a->b, Copy b->c).
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src } = inst {
                    if non_gpr_values.contains(&dest.0) {
                        continue;
                    }
                    let src_is_non_gpr = match src {
                        Operand::Value(v) => non_gpr_values.contains(&v.0),
                        Operand::Const(IrConst::F32(_))
                        | Operand::Const(IrConst::F64(_))
                        | Operand::Const(IrConst::LongDouble(..))
                        | Operand::Const(IrConst::I128(_)) => true,
                        Operand::Const(IrConst::I64(_)) if is_32bit => true,
                        _ => false,
                    };
                    if src_is_non_gpr {
                        non_gpr_values.insert(dest.0);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    non_gpr_values
}

/// Remove values from the eligible set that are used as operands in instructions
/// whose codegen paths use resolve_slot_addr() directly (not register-aware).
/// This includes CallIndirect func pointers, Memcpy pointers, va_arg pointers,
/// atomic pointers, StackRestore, and InlineAsm operands.
fn remove_ineligible_operands(
    func: &IrFunction,
    eligible: &mut FxHashSet<u32>,
    config: &RegAllocConfig,
) {
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
                    eligible.remove(&dest.0);
                    eligible.remove(&src.0);
                }
                Instruction::VaArg { va_list_ptr, .. } => {
                    eligible.remove(&va_list_ptr.0);
                }
                Instruction::VaStart { va_list_ptr } => {
                    eligible.remove(&va_list_ptr.0);
                }
                Instruction::VaEnd { va_list_ptr } => {
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
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::AtomicInc {
                    ptr: Operand::Value(v),
                    ..
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::AtomicCmpxchg {
                    ptr: Operand::Value(v),
                    ..
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::AtomicLoad {
                    ptr: Operand::Value(v),
                    ..
                } => {
                    eligible.remove(&v.0);
                }
                Instruction::AtomicStore {
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
                        // Inline asm operands are accessed via stack slots
                        // in codegen. Exclude them from register allocation
                        // unless the backend's asm emitter checks reg_assignments.
                        for (_, val, _) in outputs {
                            eligible.remove(&val.0);
                        }
                        for (_, op, _) in inputs {
                            if let Operand::Value(v) = op {
                                eligible.remove(&v.0);
                            }
                        }
                    }
                    // When allow_inline_asm_regalloc is true (RISC-V), the
                    // asm emitter checks reg_assignments before falling back
                    // to stack slot access.
                }
                _ => {}
            }
        }
    }
}

/// Check whether a live interval spans any function call point.
/// Uses binary search since call_points is sorted by program point.
fn spans_any_call(iv: &LiveInterval, call_points: &[u32]) -> bool {
    let start_idx = call_points.partition_point(|&cp| cp < iv.start);
    start_idx < call_points.len() && call_points[start_idx] <= iv.end
}

/// Build a sorted list of allocation candidates from live intervals.
///
/// Filters by eligibility, minimum span length, and call-spanning behavior:
/// - `spans_call == Some(true)`: only intervals that span a call
/// - `spans_call == Some(false)`: only intervals that do NOT span a call
/// - `spans_call == None`: all eligible intervals
///
/// Results are sorted by weighted use count (descending), with interval length
/// as tiebreaker.
fn build_sorted_candidates<'a>(
    liveness: &'a LivenessResult,
    eligible: &FxHashSet<u32>,
    already_assigned: &FxHashMap<u32, PhysReg>,
    call_points: &[u32],
    use_count: &FxHashMap<u32, u64>,
    spans_call: Option<bool>,
) -> Vec<&'a LiveInterval> {
    let mut candidates: Vec<&LiveInterval> = liveness
        .intervals
        .iter()
        .filter(|iv| eligible.contains(&iv.value_id))
        .filter(|iv| !already_assigned.contains_key(&iv.value_id))
        .filter(|iv| iv.end > iv.start)
        .filter(|iv| match spans_call {
            Some(true) => spans_any_call(iv, call_points),
            Some(false) => !spans_any_call(iv, call_points),
            None => true,
        })
        .collect();

    candidates.sort_by(|a, b| {
        let score_a = use_count.get(&a.value_id).copied().unwrap_or(1);
        let score_b = use_count.get(&b.value_id).copied().unwrap_or(1);
        score_b.cmp(&score_a).then_with(|| {
            let len_a = (a.end - a.start) as u64;
            let len_b = (b.end - b.start) as u64;
            len_b.cmp(&len_a)
        })
    });

    candidates
}

/// Find the best callee-saved register for an interval, preferring registers
/// that are already in use (to minimize prologue/epilogue save/restore cost).
///
/// Returns the index into `available_regs` of the chosen register, or None
/// if no register is free at the interval's start point.
fn find_best_callee_reg(
    reg_free_until: &[u32],
    interval_start: u32,
    available_regs: &[PhysReg],
    used_regs_set: &FxHashSet<u8>,
) -> Option<usize> {
    let mut best_already_used: Option<usize> = None;
    let mut best_already_used_free_time: u32 = u32::MAX;
    let mut best_new: Option<usize> = None;
    let mut best_new_free_time: u32 = u32::MAX;

    for (i, &free_until) in reg_free_until.iter().enumerate() {
        if free_until <= interval_start {
            let reg_id = available_regs[i].0;
            if used_regs_set.contains(&reg_id) {
                // Already saved/restored — reusing costs nothing extra.
                if best_already_used.is_none() || free_until < best_already_used_free_time {
                    best_already_used = Some(i);
                    best_already_used_free_time = free_until;
                }
            } else {
                // Would introduce a new callee-saved register.
                if best_new.is_none() || free_until < best_new_free_time {
                    best_new = Some(i);
                    best_new_free_time = free_until;
                }
            }
        }
    }

    best_already_used.or(best_new)
}

/// Exclude every 3rd fusible multiply temp from register allocation.
///
/// This creates a 3-channel multiply ILP pattern:
/// - Channel 1: register-allocated temp (e.g., r12) via standard path
/// - Channel 2: register-allocated temp (e.g., rbx) via standard path
/// - Channel 3: unregistered temp → accumulator path (%eax) via mul-add fusion
///
/// With 3 independent multiply chains, the CPU can fully utilize the multiply
/// port's throughput (1 imul/cycle) despite its 3-cycle latency.
fn exclude_every_third_mul_temp(func: &IrFunction, eligible: &mut FxHashSet<u32>) {
    // Count uses per value
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

    // Collect fusible multiply temps in program order
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

    // Only apply the 3-channel pattern when there are enough fusible temps
    // to benefit from ILP (at least 6 = two full rotations).
    if fusible_temps.len() < 6 {
        return;
    }

    // Exclude every 3rd temp (indices 2, 5, 8, 11, ...) from register allocation.
    // These will use the accumulator path (%eax) via multiply-add fusion.
    for (i, &temp_id) in fusible_temps.iter().enumerate() {
        if i % 3 == 2 {
            eligible.remove(&temp_id);
        }
    }
}

/// Count weighted uses per value in loop blocks.
/// Returns a map: value_id -> weighted_use_count (uses * 10^loop_depth).
fn count_value_uses_in_loop(func: &IrFunction, block_loop_depth: &[u32]) -> FxHashMap<u32, u64> {
    let mut uses: FxHashMap<u32, u64> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let depth = block_loop_depth.get(block_idx).copied().unwrap_or(0);
        if depth == 0 {
            continue;
        }
        let weight = match depth {
            1 => 10u64,
            2 => 100,
            3 => 1000,
            _ => 10_000,
        };
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *uses.entry(v.0).or_insert(0) += weight;
                }
            });
        }
    }
    uses
}

/// Detect phi coalesce groups for loop-carried variables.
///
/// After phi elimination, loop-header phi nodes become Copy instructions in
/// predecessor blocks. For the backedge predecessor, this creates a Copy:
///   `%phi_dest = copy %backedge_src`
/// where `%phi_dest` is the multi-def phi variable and `%backedge_src` is the
/// new value computed in the loop body.
///
/// By coalescing these two values (giving them the same register), the Copy
/// becomes a no-op, eliminating a register-to-register move or stack round-trip.
///
/// Returns a list of (phi_dest, backedge_src) pairs that should share a register.
fn detect_phi_coalesce_groups(func: &IrFunction, liveness: &LivenessResult) -> Vec<(u32, u32)> {
    // Step 1: Find multi-def values (phi dests after phi elimination).
    // A value is multi-def if it has Copy definitions in multiple blocks.
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    let mut multi_def: FxHashSet<u32> = FxHashSet::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::Copy { dest, .. } = inst {
                if let Some(&prev) = def_block.get(&dest.0) {
                    if prev != block_idx {
                        multi_def.insert(dest.0);
                    }
                }
                def_block.insert(dest.0, block_idx);
            }
        }
    }

    if multi_def.is_empty() {
        return Vec::new();
    }

    // Step 1b: Build use-block map for backedge source safety check.
    // If a backedge source is used in blocks OTHER than the Copy's block,
    // coalescing is unsafe: the source's register would be reused by the
    // allocator for other values in those blocks, clobbering the source
    // before its cross-block uses.
    let mut src_use_blocks: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            // Skip Copy dests — we care about OPERAND uses, not definitions.
            // Canonical traversal covers every instruction form (Intrinsic
            // args, Memcpy endpoints, InlineAsm inputs included).
            let mut uses = Vec::new();
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    uses.push(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| uses.push(v.0));
            for vid in uses {
                src_use_blocks.entry(vid).or_default().insert(block_idx);
            }
        }
        // Also check terminator operands
        match &block.terminator {
            Terminator::CondBranch { cond, .. } => {
                if let Operand::Value(v) = cond {
                    src_use_blocks.entry(v.0).or_default().insert(block_idx);
                }
            }
            Terminator::Return(Some(op)) => {
                if let Operand::Value(v) = op {
                    src_use_blocks.entry(v.0).or_default().insert(block_idx);
                }
            }
            Terminator::Switch { val, .. } => {
                if let Operand::Value(v) = val {
                    src_use_blocks.entry(v.0).or_default().insert(block_idx);
                }
            }
            _ => {}
        }
    }

    // Step 2: Find backedge copies in loop blocks.
    // A backedge copy is a Copy where:
    //   - The dest is a multi-def value (phi dest)
    //   - The source is a Value (not a constant)
    //   - The copy is in a block with loop_depth > 0
    let mut groups: Vec<(u32, u32)> = Vec::new();
    let mut seen_phi_dests: FxHashSet<u32> = FxHashSet::default();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let depth = liveness
            .block_loop_depth
            .get(block_idx)
            .copied()
            .unwrap_or(0);
        if depth == 0 {
            continue;
        }

        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src_val),
            } = inst
            {
                if multi_def.contains(&dest.0) && !seen_phi_dests.contains(&dest.0) {
                    // Don't coalesce if src is itself a multi-def (swap cycle temporaries)
                    if !multi_def.contains(&src_val.0) {
                        // Safety: don't coalesce if the phi dest is used AFTER
                        // the backedge source's definition. This detects the
                        // "lost copy" pattern where e.g.:
                        //   v_n = Call(malloc)       ← src defined here
                        //   Store(v_head, v_n+8)     ← phi dest USED here
                        //   Copy v_head = v_n        ← coalesce candidate
                        // Coalescing v_head and v_n to the same register would
                        // clobber v_head when storing the Call result.
                        //
                        // Important: the src may be defined in a DIFFERENT block
                        // than the Copy (multi-block loop bodies). We must check
                        // the src's defining block for phi dest uses, not just
                        // the Copy's block.
                        let mut phi_dest_used_after_src = false;

                        // Find the block that defines the backedge source
                        let mut src_def_block = None;
                        for (bi, b) in func.blocks.iter().enumerate() {
                            for i in &b.instructions {
                                if let Some(d) = i.dest() {
                                    if d.0 == src_val.0 {
                                        src_def_block = Some(bi);
                                    }
                                }
                            }
                        }

                        // Check the block containing the Copy
                        {
                            let mut src_defined = false;
                            for inst2 in &block.instructions {
                                if !src_defined {
                                    if let Some(d) = inst2.dest() {
                                        if d.0 == src_val.0 {
                                            src_defined = true;
                                        }
                                    }
                                } else {
                                    if let Instruction::Copy { dest: d, .. } = inst2 {
                                        if d.0 == dest.0 {
                                            break;
                                        }
                                    }
                                    if uses_value(inst2, dest.0) {
                                        phi_dest_used_after_src = true;
                                    }
                                }
                            }
                        }

                        // If the src is defined in a DIFFERENT block, also check
                        // that block (and any other block the src's value flows
                        // through) for phi dest uses after the src definition.
                        if let Some(sdb) = src_def_block {
                            if sdb != block_idx {
                                let mut src_defined = false;
                                for inst2 in &func.blocks[sdb].instructions {
                                    if !src_defined {
                                        if let Some(d) = inst2.dest() {
                                            if d.0 == src_val.0 {
                                                src_defined = true;
                                            }
                                        }
                                    } else {
                                        if uses_value(inst2, dest.0) {
                                            phi_dest_used_after_src = true;
                                        }
                                    }
                                }
                            }
                        }
                        // Also check: the backedge source must not have uses
                        // in OTHER blocks. If it does, coalescing gives it the
                        // phi dest's register, but the allocator may reassign
                        // that register to other values in those blocks,
                        // clobbering the source before its cross-block uses.
                        let src_has_cross_block_use = src_use_blocks
                            .get(&src_val.0)
                            .map(|blocks| blocks.iter().any(|&b| b != block_idx))
                            .unwrap_or(false);

                        if !phi_dest_used_after_src && !src_has_cross_block_use {
                            if std::env::var("CCC_DEBUG_PHI_COALESCE").is_ok() {
                                eprintln!("[PHI_COALESCE] Coalescing phi_dest=Value({}) with backedge_src=Value({}) in block {}",
                                    dest.0, src_val.0, block_idx);
                            }
                            groups.push((dest.0, src_val.0));
                            seen_phi_dests.insert(dest.0);
                        } else if std::env::var("CCC_DEBUG_PHI_COALESCE").is_ok() {
                            eprintln!("[PHI_COALESCE] BLOCKED phi_dest=Value({}) with backedge_src=Value({}) in block {} (used_after={}, cross_block={})",
                                dest.0, src_val.0, block_idx, phi_dest_used_after_src, src_has_cross_block_use);
                        }
                    }
                }
            }
        }
    }

    groups
}

/// Check if an instruction uses a given value ID as an operand (not as dest).
/// Uses the canonical operand/value traversal so EVERY instruction form is
/// covered (Intrinsic args, Memcpy endpoints, InlineAsm inputs, atomics...).
/// A hand-maintained match previously missed `Intrinsic` args, letting phi
/// coalescing merge a pointer phi with its backedge increment while the phi
/// was still live as an intrinsic operand (zlib-ng adler32_avx2 miscompile:
/// the in-place `addq $32` clobbered the load address).
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
