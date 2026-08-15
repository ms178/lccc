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
    // The AArch64 FP pool (allocator IDs 40+) additionally keeps scalar
    // float intrinsic results out of GPRs so they can use FP registers.
    let arm_fp_pool = config.xmm_regs.first().is_some_and(|r| r.0 == 40)
        && std::env::var("CCC_NO_VECREG").is_err();
    let non_gpr_values = collect_non_gpr_values(func, is_32bit, arm_fp_pool);

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
    //
    // This is an x86-64-specific trick: on AArch64/RISC-V the accumulator
    // path is a single register and excluding mul temps from registers only
    // adds shuffle overhead, so it is gated to the x86-64 register pool.
    if config.xmm_regs.first().is_some_and(|r| r.0 == 20) {
        exclude_every_third_mul_temp(func, &mut eligible);
    }

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
    let all_phi_pairs = detect_phi_coalesce_groups(func, &liveness);
    let mut phi_coalesce = if std::env::var("CCC_NO_PHI_COALESCE").is_ok() {
        Vec::new()
    } else {
        all_phi_pairs.clone()
    };
    for candidate in &phi_coalesce {
        // Remove the backedge source from independent allocation. If the
        // candidate survives final conflict checks it inherits the phi
        // destination's register.
        eligible.remove(&candidate.backedge_src);
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
    // previous baseline.
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

    // --- Post-scan rebalance: steal registers for hot loop-carried values ---
    //
    // The linear scan processes ranges in start order and never evicts, so
    // cold function-spanning values (array bases, globals, prologue pointers)
    // win every callee-saved register simply by starting first, leaving hot
    // inner-loop-carried phi values (IVs, accumulators, carried pointers)
    // stack-homed (e.g. fannkuch's flip loop, arith_loop).
    //
    // For each hot loop-carried phi dest the scan MISSED, pick the register
    // whose conflicting holders (live interval overlaps the hot value's) have
    // the coldest total use count, and fully deallocate those holders back to
    // the stack. This is safe where eviction inside the scan was not: an
    // evicted holder is deallocated for its ENTIRE interval — indistinguishable
    // from never having been assigned, so the default stack path handles all
    // its uses — and every remaining holder is provably non-overlapping with
    // the hot value. No live range is ever split, and when the scan already
    // housed every hot value (e.g. spectral_norm) the rebalance is a no-op.
    //
    // AArch64-only: relies on the wide callee-saved GPR pool.
    // CCC_NO_LOOP_PIN disables; CCC_LOOP_PIN=N caps steals per function (default 2).
    if std::env::var("CCC_NO_LOOP_PIN").is_err()
        && config.xmm_regs.first().is_some_and(|r| r.0 == 40)
        && !all_phi_pairs.is_empty()
    {
        let k: usize = std::env::var("CCC_LOOP_PIN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        // Values whose register assignment the phi-coalesce propagation below
        // depends on; stealing from them would defeat coalescing.
        let phi_pair_values: FxHashSet<u32> = phi_coalesce
            .iter()
            .flat_map(|c| [c.phi_dest, c.backedge_src])
            .collect();
        // Rank loop-carried phi dests by loop-weighted use count (uses inside
        // a loop at depth D contribute 10^D, so hot inner-loop values sort first).
        let mut candidates: Vec<(u32, u64)> = all_phi_pairs
            .iter()
            .map(|c| c.phi_dest)
            .filter(|v| eligible.contains(v))
            .filter(|v| {
                liveness
                    .intervals
                    .iter()
                    .any(|iv| iv.value_id == *v && iv.end > iv.start)
            })
            .map(|v| (v, use_count.get(&v).copied().unwrap_or(0)))
            .filter(|&(_, count)| count >= 10) // at least one in-loop use
            .collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        candidates.dedup_by_key(|&mut (v, _)| v);

        // All live-interval segments per value; conflict checks must consider
        // every segment, not just the first.
        let mut segs_of: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
        for iv in &liveness.intervals {
            segs_of.entry(iv.value_id).or_default().push((iv.start, iv.end));
        }
        let overlaps = |a: u32, b: u32| -> bool {
            let (Some(sa), Some(sb)) = (segs_of.get(&a), segs_of.get(&b)) else {
                return false;
            };
            sa.iter().any(|&(s1, e1)| sb.iter().any(|&(s2, e2)| s1 < e2 && s2 < e1))
        };
        // Current register holders, updated as steals happen.
        let mut holders_by_reg: FxHashMap<u8, Vec<u32>> = FxHashMap::default();
        for (&v, &r) in &assignments {
            holders_by_reg.entry(r.0).or_default().push(v);
        }

        let mut steals = 0;
        for &(vid, hot_count) in &candidates {
            if steals >= k {
                break;
            }
            if assignments.contains_key(&vid) {
                continue; // the scan already housed this hot value
            }
            if std::env::var("CCC_DEBUG_LOOP_PIN").is_ok() {
                eprintln!("[LOOP_PIN] func={} candidate v{} (use {}) MISSED by scan", func.name, vid, hot_count);
            }
            // Choose the register whose CONFLICTING holders are coldest.
            // Every holder whose live interval overlaps the hot value must be
            // fully deallocated back to the stack for the steal to be sound;
            // non-conflicting holders keep timesharing the register. Registers
            // holding phi-coalesce participants are left alone so backedge
            // coalescing stays intact. A steal is only taken when the total
            // evicted use count is strictly colder than the hot value.
            let mut best: Option<(u8, Vec<u32>, u64)> = None;
            for (&reg_id, holders) in &holders_by_reg {
                if holders.iter().any(|h| phi_pair_values.contains(h)) {
                    continue;
                }
                let evict: Vec<u32> = holders
                    .iter()
                    .copied()
                    .filter(|&h| overlaps(h, vid))
                    .collect();
                let cost: u64 = evict
                    .iter()
                    .map(|h| use_count.get(h).copied().unwrap_or(0))
                    .sum();
                if cost >= hot_count {
                    continue; // not profitable
                }
                if best.as_ref().map_or(true, |&(_, _, c)| cost < c) {
                    best = Some((reg_id, evict, cost));
                }
            }
            if let Some((reg_id, evict, cost)) = best {
                if std::env::var("CCC_DEBUG_LOOP_PIN").is_ok() {
                    eprintln!(
                        "[LOOP_PIN] func={} v{} (use {}) takes reg {}, evicting {:?} (use {})",
                        func.name, vid, hot_count, reg_id, evict, cost
                    );
                }
                for v in &evict {
                    assignments.remove(v);
                    if let Some(h) = holders_by_reg.get_mut(&reg_id) {
                        h.retain(|x| x != v);
                    }
                }
                assignments.insert(vid, PhysReg(reg_id));
                holders_by_reg.entry(reg_id).or_default().push(vid);
                steals += 1;
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
    // Caller-saved registers allocated in Phase 2. These do NOT belong in
    // used_regs_set (no prologue save), but Phase 2b must not reallocate them.
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

    // Propagate proven phi-coalesce assignments. A candidate carries its
    // exact same-block definition/copy window from detection; revalidate that
    // window before changing locations so future refactors fail closed.
    for candidate in &phi_coalesce {
        let phi_dest = candidate.phi_dest;
        let backedge_src = candidate.backedge_src;
        let Some(block) = func.blocks.get(candidate.block_idx) else {
            continue;
        };
        if candidate.source_def_idx >= candidate.copy_idx
            || candidate.copy_idx >= block.instructions.len()
            || block.instructions[candidate.source_def_idx]
                .dest()
                .is_none_or(|dest| dest.0 != backedge_src)
            || !matches!(
                block.instructions[candidate.copy_idx],
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } if dest.0 == phi_dest && src.0 == backedge_src
            )
            || block.instructions[candidate.source_def_idx + 1..candidate.copy_idx]
                .iter()
                .any(|inst| uses_value(inst, phi_dest))
        {
            continue;
        }

        let Some(&reg) = assignments.get(&phi_dest) else {
            continue;
        };
        let src_interval = liveness
            .intervals
            .iter()
            .find(|iv| iv.value_id == backedge_src);
        if let Some(src_iv) = src_interval {
            // A value already allocated to this register must not overlap the
            // source interval. The phi destination itself is intentionally
            // excluded: the same-block window proof above establishes the
            // legal destructive update from old phi value to new source value.
            let has_conflict = liveness.intervals.iter().any(|iv| {
                if iv.value_id == backedge_src || iv.value_id == phi_dest {
                    return false;
                }
                assignments.get(&iv.value_id).is_some_and(|other_reg| {
                    other_reg.0 == reg.0 && iv.start < src_iv.end && src_iv.start < iv.end
                })
            });
            if has_conflict {
                continue;
            }
        }
        assignments.insert(backedge_src, reg);
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

    // AArch64 (allocator IDs 40..47 → v16..v23) additionally allocates
    // 128-bit vector values and copy-form F64 values (loop accumulators)
    // to FP/SIMD registers; other targets keep those stack-homed because
    // their emitters are not register-aware for them.
    let (vector_values, f64_value_set) = if arm_fp_pool {
        (collect_vector_values(func), collect_f64_values(func))
    } else {
        (FxHashSet::default(), FxHashSet::default())
    };

    // Phase 3: XMM/FP register allocation for scalar FP (F64/F32) values
    // that don't span calls. These values were excluded from GPR allocation
    // but can use XMM (x86) / v-register (AArch64) homes.
    if !config.xmm_regs.is_empty() {
        // Values actually consumed by a real (non-Copy) instruction.  SSA copy
        // webs can carry a value across loop boundaries without it ever being
        // used in a computation; such values would otherwise win FP registers
        // with their huge intervals and starve the real accumulators.
        // A copy source feeding (transitively) a real use still qualifies —
        // e.g. an fmadd result copied into a loop accumulator.
        let mut real_use: FxHashSet<u32> = FxHashSet::default();
        if arm_fp_pool {
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
            loop {
                let mut changed = false;
                for block in &func.blocks {
                    for inst in &block.instructions {
                        if let Instruction::Copy { dest, src: Operand::Value(src_val) } = inst {
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
        // Collect F64 values: values in non_gpr_values that are F64 typed,
        // haven't been assigned a GPR, and don't span calls.
        let f64_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| non_gpr_values.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| !spans_any_call(iv, call_points))
            // Skip values that are only ever copied (never feed a computation):
            // they don't need a register and would starve values that do.
            .filter(|iv| !arm_fp_pool || real_use.contains(&iv.value_id))
            // Only include values that are actually scalar F64/F32 (not i128,
            // not long double, etc.) — plus (AArch64) 128-bit vector values
            // and copy-form F64 values.
            .filter(|iv| {
                vector_values.contains(&iv.value_id) || f64_value_set.contains(&iv.value_id) || {
                    if arm_fp_pool {
                        return false;
                    }
                    // x86: check if this value is produced by an F64/F32-typed
                    // instruction (or a scalar FP intrinsic such as sqrt/fabs).
                    func.blocks.iter().any(|block| {
                        block.instructions.iter().any(|inst| match inst {
                            Instruction::BinOp { dest, ty, .. }
                            | Instruction::UnaryOp { dest, ty, .. }
                                if *ty == IrType::F64 || *ty == IrType::F32 =>
                            {
                                dest.0 == iv.value_id
                            }
                            Instruction::Load { dest, ty, .. }
                                if *ty == IrType::F64 || *ty == IrType::F32 =>
                            {
                                dest.0 == iv.value_id
                            }
                            Instruction::ParamRef { dest, ty, .. }
                                if *ty == IrType::F64 || *ty == IrType::F32 =>
                            {
                                dest.0 == iv.value_id
                            }
                            Instruction::Cast { dest, to_ty, .. }
                                if *to_ty == IrType::F64 || *to_ty == IrType::F32 =>
                            {
                                dest.0 == iv.value_id
                            }
                            Instruction::Intrinsic { dest: Some(d), op, .. } => {
                                use crate::ir::intrinsics::IntrinsicOp as O;
                                d.0 == iv.value_id
                                    && matches!(
                                        op,
                                        O::SqrtF64 | O::FabsF64 | O::SqrtF32 | O::FabsF32
                                    )
                            }
                            _ => false,
                        })
                    })
                }
            })
            .copied()
            .collect();

        if std::env::var("CCC_DEBUG_VECREG").is_ok() {
            eprintln!("[VECREG] func={} vector_values={:?}", func.name, vector_values);
            for &vid in &vector_values {
                let iv = liveness.intervals.iter().find(|iv| iv.value_id == vid);
                eprintln!("[VECREG]   v{}: interval={:?} non_gpr={} assigned={} spans_call={}",
                    vid, iv.map(|i| (i.start, i.end)), non_gpr_values.contains(&vid),
                    assignments.contains_key(&vid),
                    iv.is_some_and(|i| spans_any_call(i, call_points)));
            }
        }
        if std::env::var("CCC_DEBUG_FPREG").is_ok() {
            eprintln!("[FPREG] func={} f64_count={} intervals_in={}", func.name, f64_value_set.len(), f64_intervals.len());
            for iv in &f64_intervals {
                eprintln!("[FPREG]   cand v{} [{}, {}]", iv.value_id, iv.start, iv.end);
            }
            for &vid in &f64_value_set {
                if liveness.intervals.iter().all(|iv| iv.value_id != vid) {
                    eprintln!("[FPREG]   v{}: NO INTERVAL", vid);
                }
            }
            for iv in &liveness.intervals {
                if f64_value_set.contains(&iv.value_id) && f64_intervals.iter().all(|c| c.value_id != iv.value_id) {
                    eprintln!("[FPREG]   excluded v{} [{}, {}] non_gpr={} assigned={} spans_call={}",
                        iv.value_id, iv.start, iv.end, non_gpr_values.contains(&iv.value_id),
                        assignments.contains_key(&iv.value_id), spans_any_call(iv, call_points));
                }
            }
        }

        if std::env::var("CCC_DEBUG_XMM").is_ok() {
            eprintln!(
                "[XMM] fn={} f64_intervals={} regs={:?}",
                func.name,
                f64_intervals.len(),
                config.xmm_regs.iter().map(|r| r.0).collect::<Vec<_>>()
            );
            for iv in &f64_intervals {
                eprintln!("[XMM]   val{} [{},{}]", iv.value_id, iv.start, iv.end);
            }
        }

        if !f64_intervals.is_empty() {
            let f64_ranges =
                live_range::build_live_ranges(&f64_intervals, &liveness.block_loop_depth, func);
            let mut xmm_allocator = LinearScanAllocator::new(f64_ranges, config.xmm_regs.clone());
            xmm_allocator.run();

            if std::env::var("CCC_DEBUG_XMM").is_ok() {
                eprintln!(
                    "[XMM]   assigned={:?}",
                    xmm_allocator.assignments.iter().map(|(v, r)| (*v, r.0)).collect::<Vec<_>>()
                );
            }

            for (vid, reg) in &xmm_allocator.assignments {
                if std::env::var("CCC_DEBUG_VECREG").is_ok() && vector_values.contains(vid) {
                    eprintln!("[VECREG]   assigned v{} -> reg {}", vid, reg.0);
                }
                if std::env::var("CCC_DEBUG_FPREG").is_ok() {
                    eprintln!("[FPREG]   assigned v{} -> reg {} (pool size {})", vid, reg.0, config.xmm_regs.len());
                }
            }
            for (vid, reg) in xmm_allocator.assignments {
                assignments.insert(vid, reg);
                // XMM regs (20+) are caller-saved, no prologue save needed
            }

            // FP producer->consumer coalescing now happens DURING allocation
            // via the linear-scan follow hints (see live_range.rs); the
            // post-allocation pass is gone. The hint is honoured under the
            // same die-at-birth conflict predicate the old pass used, but is
            // no longer restricted to single-use producers.
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

    // AArch64 reserves allocator IDs 48..55 for explicitly marked loop-carried
    // F64 values (d24..d31). They are caller-saved and disjoint from the generic
    // d16..d23 pool, so reductions do not reduce temporary-register capacity.
    if config.xmm_regs.first().is_some_and(|r| r.0 == 40) {
        for (index, value) in func.loop_promoted_f64_values.iter().take(8).enumerate() {
            assignments.insert(value.0, PhysReg(48 + index as u8));
        }
    }

    // AArch64 FP phi coalescing: give the value copied into a loop-carried
    // F64/vector accumulator at the backedge the accumulator's own register,
    // eliminating the backedge fmov/mov from the loop's serial dependency
    // chain (e.g. `fmadd d16, .., d16` instead of fmadd into a temp + fmov).
    if arm_fp_pool {
        for candidate in &all_phi_pairs {
            let phi_dest = candidate.phi_dest;
            let backedge_src = candidate.backedge_src;
            // FP pool spans caller-saved d16..d31 (40..=55) AND callee-saved
            // d8..d14 (32..=38) — both are valid coalesce targets.
            let is_fp = |r: &PhysReg| (32..=38).contains(&r.0) || (40..=55).contains(&r.0);
            let d_reg = assignments.get(&phi_dest).copied().filter(is_fp);
            let s_reg = assignments.get(&backedge_src).copied().filter(is_fp);
            let (Some(d), Some(s)) = (d_reg, s_reg) else { continue };
            if d == s {
                continue;
            }
            if !f64_value_set.contains(&phi_dest) && !vector_values.contains(&phi_dest) {
                continue;
            }
            // Conflict check: no other value assigned d may overlap the src
            // interval (the phi dest itself is expected to overlap — that is
            // precisely what coalescing resolves).
            if let Some(src_iv) = liveness.intervals.iter().find(|iv| iv.value_id == backedge_src) {
                let conflict = liveness.intervals.iter().any(|iv| {
                    if iv.value_id == backedge_src || iv.value_id == phi_dest {
                        return false;
                    }
                    assignments
                        .get(&iv.value_id)
                        .is_some_and(|&o| o.0 == d.0 && iv.start < src_iv.end && src_iv.start < iv.end)
                });
                if std::env::var("CCC_DEBUG_FPCOAL").is_ok() {
                    eprintln!("[FPCOAL] phi={} (d{}) src={} (d{}) conflict={}", phi_dest, d.0, backedge_src, s.0, conflict);
                }
                if !conflict {
                    assignments.insert(backedge_src, d);
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
            // Defensive: never hand out a register the prologue already
            // treats as callee-saved (cannot happen for a well-formed config
            // where the pools are disjoint, but harmless and cheap to check).
            .filter(|r| !used_regs_set.contains(&r.0))
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

/// CCC_ENABLE_VECREG: collect the 128-bit VECTOR VALUES that are safe to
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
            | O::Pblendvb128 | O::Pblendw128
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
            | O::VecLoadF32x4 | O::VecLoadF32x8
            | O::VecAddF64x2 | O::VecAddF64x4 | O::VecAddI32x4 | O::VecAddI32x8
            | O::VecAddF32x4 | O::VecAddF32x8
            | O::VecMulF64x2 | O::VecMulF64x4 | O::VecMulF32x4 | O::VecMulF32x8
            | O::VecHorizontalAddF64x2 | O::VecHorizontalAddF64x4
            | O::VecHorizontalAddI32x4 | O::VecHorizontalAddI32x8
            | O::VecHorizontalAddF32x4 | O::VecHorizontalAddF32x8
            | O::VecZeroF32x4 | O::VecZeroF32x8
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
fn collect_non_gpr_values(func: &IrFunction, is_32bit: bool, arm_fp_pool: bool) -> FxHashSet<u32> {
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
                Instruction::ParamRef { dest, ty, .. } => {
                    // An FP (or i128/long-double) parameter arrives in an XMM
                    // register per the ABI. Marking its ParamRef dest non-GPR
                    // makes it eligible for the Phase 3 XMM scan, so a hot
                    // call-free function keeps the parameter in an XMM
                    // register instead of spilling it to a slot at entry and
                    // reloading it on every use.
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
                    // Scalar FP intrinsics (sqrt/fabs) produce F64/F32 values
                    // that must live in XMM/FP registers, never GPRs. Vector
                    // intrinsics produce 128/256-bit values that also cannot
                    // be stored in scalar GPRs — exclude both from GPR
                    // allocation. produces_vector_value() is the single
                    // source of truth for the vector set (covers x86 Vec*
                    // and the AArch64 I64x2 widening family).
                    use crate::ir::intrinsics::IntrinsicOp;
                    if matches!(
                        op,
                        IntrinsicOp::SqrtF64 | IntrinsicOp::SqrtF32
                            | IntrinsicOp::FabsF64 | IntrinsicOp::FabsF32
                    ) || op.produces_vector_value()
                    {
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

/// Collect SSA values that hold 128/256-bit vector data: destinations of
/// vector-producing intrinsics plus any Copy destinations whose source is a
/// vector value (iterated to fixpoint, mirroring the non-GPR propagation).
/// Used on AArch64 to allocate NEON registers to vector values (the Phase 3
/// FP/vector pool maps allocator IDs 40..47 to v16..v23).
fn collect_vector_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut vector_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic { dest: Some(d), op, .. } = inst {
                if op.produces_vector_value() {
                    vector_values.insert(d.0);
                }
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src: Operand::Value(src_val) } = inst {
                    if !vector_values.contains(&dest.0) && vector_values.contains(&src_val.0) {
                        vector_values.insert(dest.0);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    vector_values
}

/// Collect SSA values that hold scalar F64 data: destinations of F64-typed
/// instructions (BinOp/UnaryOp/Load/Cast), F64-returning intrinsics, and
/// F64 constants — plus, iteratively, any Copy whose source is F64.  The copy
/// propagation is what makes loop-carried F64 accumulators (lowered to Copy
/// form after phi elimination) visible to the Phase 3 FP register scan.
fn collect_f64_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut f64_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Load { dest, ty, .. } if *ty == IrType::F64 => {
                    f64_values.insert(dest.0);
                }
                Instruction::Cast { dest, to_ty, .. } if *to_ty == IrType::F64 => {
                    f64_values.insert(dest.0);
                }
                Instruction::Copy { dest, src: Operand::Const(IrConst::F64(_)) } => {
                    f64_values.insert(dest.0);
                }
                Instruction::Intrinsic { dest: Some(d), op, .. }
                    if matches!(op, crate::ir::intrinsics::IntrinsicOp::SqrtF64
                        | crate::ir::intrinsics::IntrinsicOp::FabsF64) =>
                {
                    f64_values.insert(d.0);
                }
                _ => {}
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src: Operand::Value(src_val) } = inst {
                    if !f64_values.contains(&dest.0) && f64_values.contains(&src_val.0) {
                        f64_values.insert(dest.0);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    f64_values
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

/// A phi/backedge pair that may share one physical register.
///
/// `source_def_idx..copy_idx` is a straight-line window in `block_idx`.
/// Keeping these exact sites makes the destructive update proof explicit and
/// lets assignment propagation revalidate it without whole-function searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhiCoalesceCandidate {
    pub(crate) phi_dest: u32,
    pub(crate) backedge_src: u32,
    pub(crate) block_idx: usize,
    pub(crate) source_def_idx: usize,
    pub(crate) copy_idx: usize,
}

/// Detect safe phi coalesce candidates for loop-carried variables.
///
/// After phi elimination, a backedge contains `%phi = copy %next`. Sharing a
/// register removes that copy, but it is a destructive update: `%next`'s
/// definition overwrites `%phi`. This is safe only when definition and Copy are
/// in the SAME basic block and `%phi` is not read between them. If the source is
/// defined in an earlier block, an intervening successor can still read the old
/// phi value (SQLite deleteTable); checking only the source and Copy blocks
/// cannot prove otherwise.
///
/// Also used by stack-layout copy coalescing: the same proof (phi dest not used
/// after the backedge source is defined) makes sharing a *stack slot* safe.
pub(crate) fn detect_phi_coalesce_groups(
    func: &IrFunction,
    liveness: &LivenessResult,
) -> Vec<PhiCoalesceCandidate> {
    // Build both forms of definition metadata in one pass:
    // - multi_def identifies post-phi destinations defined by edge Copies;
    // - unique_def_site proves a source has exactly one definition and records
    //   its exact straight-line location. None means multiple definitions.
    let mut first_copy_def_block: FxHashMap<u32, usize> = FxHashMap::default();
    let mut multi_def: FxHashSet<u32> = FxHashSet::default();
    let mut unique_def_site: FxHashMap<u32, Option<(usize, usize)>> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                unique_def_site
                    .entry(dest.0)
                    .and_modify(|site| *site = None)
                    .or_insert(Some((block_idx, inst_idx)));
            }
            if let Instruction::Copy { dest, .. } = inst {
                if first_copy_def_block
                    .insert(dest.0, block_idx)
                    .is_some_and(|previous| previous != block_idx)
                {
                    multi_def.insert(dest.0);
                }
            }
        }
    }
    if multi_def.is_empty() {
        return Vec::new();
    }

    // Source uses outside the Copy block make destructive coalescing unsafe.
    // Canonical visitors cover data operands, pointer-only uses, and every
    // terminator form without hand-maintained omissions.
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

    let debug = std::env::var("CCC_DEBUG_PHI_COALESCE").is_ok();
    let mut candidates = Vec::new();
    // A phi dest may have several loop-block copies (loop-entry initialization
    // plus the true backedge update, or multiple latches via `continue`). ALL
    // pairs are returned: keeping only the first found meant an entry copy
    // shadowed the true backedge copy, so the accumulator update kept a
    // register shuffle in the loop. Safety relies on the consumers' own
    // proofs: the same-block window revalidation + interval conflict check in
    // register propagation (later pairs see earlier pairs' assignments and
    // fail closed), and the one-NEW-alias-per-dest claim set in slot
    // coalescing (claimed_dests).

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

            // Core SQLite deleteTable soundness rule. A basic block is the only
            // region with no hidden control-flow path between definition and
            // Copy; different blocks require full path-sensitive interference
            // and are conservatively not coalesced.
            if source_block != block_idx || source_def_idx >= copy_idx {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}): def block/index {}:{} != copy {}:{}",
                        dest.0, src.0, source_block, source_def_idx, block_idx, copy_idx
                    );
                }
                continue;
            }

            let phi_used_in_window = block.instructions[source_def_idx + 1..copy_idx]
                .iter()
                .any(|middle| uses_value(middle, dest.0));
            let source_used_elsewhere = src_use_blocks
                .get(&src.0)
                .is_some_and(|blocks| blocks.iter().any(|&use_block| use_block != block_idx));
            if phi_used_in_window || source_used_elsewhere {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}) block={} used_in_window={} cross_block={}",
                        dest.0, src.0, block_idx, phi_used_in_window, source_used_elsewhere
                    );
                }
                continue;
            }

            if debug {
                eprintln!(
                    "[PHI_COALESCE] Coalescing phi_dest=Value({}) with backedge_src=Value({}) in block {} window {}..{}",
                    dest.0, src.0, block_idx, source_def_idx, copy_idx
                );
            }
            candidates.push(PhiCoalesceCandidate {
                phi_dest: dest.0,
                backedge_src: src.0,
                block_idx,
                source_def_idx,
                copy_idx,
            });
        }
    }

    candidates
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
#[cfg(test)]
mod phi_coalesce_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Value};

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
        // Coalescing source and phi destination clobbers the old value before
        // block 3. Looking only in the source and Copy blocks misses the use.
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
}
