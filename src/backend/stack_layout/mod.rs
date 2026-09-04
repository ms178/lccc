//! Stack layout: slot assignment, alloca coalescing, and regalloc helpers.
//!
//! ## Architecture
//!
//! Stack space calculation uses a three-tier allocation scheme:
//!
//! - **Tier 1**: Allocas get permanent, non-shared slots (addressable memory).
//!   Exception: non-escaping single-block allocas use Tier 3 sharing.
//!
//! - **Tier 2**: Multi-block non-alloca SSA temporaries use liveness-based packing.
//!   Values with non-overlapping live intervals share the same stack slot,
//!   using a greedy interval coloring algorithm via a min-heap.
//!
//! - **Tier 3**: Single-block non-alloca values use block-local coalescing with
//!   intra-block greedy slot reuse. Each block has its own pool; pools from
//!   different blocks overlap since only one block executes at a time.
//!
//! ## Submodules
//!
//! - `analysis`: value use-block maps, used-value collection, dead param detection
//! - `alloca_coalescing`: escape analysis for alloca coalescability
//! - `copy_coalescing`: copy alias maps and immediately-consumed value analysis
//! - `slot_assignment`: tier classification and slot assignment (Phases 2-7)
//! - `inline_asm`: shared ASM clobber scan
//! - `regalloc_helpers`: register allocator + clobber merge, callee-saved filtering
//!
//! ## Key functions
//!
//! - `calculate_stack_space_common`: orchestrates the three-tier allocation
//! - `compute_coalescable_allocas`: escape analysis for alloca coalescing
//! - `collect_inline_asm_callee_saved`: shared ASM clobber scan
//! - `run_regalloc_and_merge_clobbers`: register allocator + clobber merge
//! - `filter_available_regs`: callee-saved register filtering
//! - `find_param_alloca`: parameter alloca lookup

mod alloca_coalescing;
mod analysis;
pub(crate) mod copy_coalescing;
mod graph_coloring;
mod inline_asm;
mod regalloc_helpers;
mod slot_assignment;

// Re-export submodule public APIs at the stack_layout:: level
pub use inline_asm::{
    collect_inline_asm_callee_saved, collect_inline_asm_callee_saved_i686,
    collect_inline_asm_callee_saved_with_generic, collect_inline_asm_callee_saved_with_overflow,
};
pub use regalloc_helpers::{
    filter_available_regs, find_param_alloca, run_regalloc_and_merge_clobbers,
    run_regalloc_and_merge_clobbers_ex,
};

use super::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrFunction};

use alloca_coalescing::CoalescableAllocas;

// ── Helper structs for slot allocation ────────────────────────────────────

/// A block-local slot whose final offset is deferred until after all tiers
/// have computed their space requirements. The final offset is:
/// `non_local_space + block_offset`.
struct DeferredSlot {
    dest_id: u32,
    size: i64,
    align: i64,
    block_offset: i64,
}

/// Multi-block value pending Tier 2 liveness-based packing.
struct MultiBlockValue {
    dest_id: u32,
    slot_size: i64,
}

/// Block-local non-alloca value pending Tier 3 intra-block reuse.
struct BlockLocalValue {
    dest_id: u32,
    slot_size: i64,
    block_idx: usize,
}

/// Intermediate state passed between phases of `calculate_stack_space_common`.
struct StackLayoutContext {
    /// Whether coalescing and multi-tier allocation is enabled (num_blocks >= 2).
    coalesce: bool,
    /// Per-value use-block map: value_id -> list of block indices where used.
    use_blocks_map: FxHashMap<u32, Vec<usize>>,
    /// Value ID -> defining block index.
    def_block: FxHashMap<u32, usize>,
    /// Values defined in multiple blocks (from phi elimination).
    multi_def_values: FxHashSet<u32>,
    /// Copy alias map: dest_id -> root_id (values sharing the same stack slot).
    copy_alias: FxHashMap<u32, u32>,
    /// Values aliased via phi-web coalescing (must force-overwrite in resolve_copy_aliases).
    phi_web_aliases: FxHashSet<u32>,
    /// Phi-web aliases certified by detect_phi_coalesce_groups (loop-backedge
    /// copies): skip the generic interference check in resolve_copy_aliases.
    loop_phi_aliases: FxHashSet<u32>,
    /// All value IDs referenced as operands in the function body.
    used_values: FxHashSet<u32>,
    /// Dead parameter allocas (unused params, skip slot allocation).
    dead_param_allocas: FxHashSet<u32>,
    /// ABI alignment of each struct/union parameter alloca (value id ->
    /// alignment), taken from the authoritative `IrParam.struct_align`. Used to
    /// recover over-aligned (>16) parameter allocas whose `Alloca.align` was
    /// dropped by a later pass — the slot must still honor the param's declared
    /// `_Alignas(32)` etc. so `&s` (and vector accesses on the param) are
    /// correctly aligned.
    param_aligns: FxHashMap<u32, usize>,
    /// Alloca coalescing analysis results.
    coalescable_allocas: CoalescableAllocas,
    /// Values that are produced and immediately consumed by the next instruction,
    /// as the first operand loaded into the accumulator. These values don't need
    /// stack slots — the accumulator register cache keeps them alive.
    immediately_consumed: FxHashSet<u32>,
    /// Values (vector-intrinsic result allocas) whose single use is args[0]/
    /// args[1] of the immediately-following intrinsic: the deferred-store
    /// set. The backend may skip their slot store entirely (accumulator
    /// renaming).
    vector_defer_values: FxHashSet<u32>,
    /// VLFOLD: 256-bit vector loads elided in favour of a folded memory
    /// operand in their adjacent consumer (`compute_vector_memfold_values`).
    vector_memfold_values: FxHashSet<u32>,
    x87_defer_values: FxHashSet<u32>,
    /// Values that appear as incoming operands in Phi instructions.
    /// These must NOT be classified as block-local (Tier 3) because phi
    /// elimination places Copies at predecessor block ends. If the source
    /// value's Tier 3 slot was reused by another block, the Copy reads garbage.
    phi_incoming_values: FxHashSet<u32>,
    /// Values that appear as the dest or src of a Memcpy, mapped to the
    /// copy width. A struct-by-value temp copied by a 32-byte Memcpy needs a
    /// 32-byte slot; the generic 8-byte fallback for Copy/Memcpy temps lets
    /// the copy read/write past its slot, corrupting neighbours
    /// (simd_avx2_256 mul_ps check: vfb copy read the add-loop residue).
    memcpy_value_sizes: FxHashMap<u32, usize>,
    /// Block indices where the block has more instructions than the Tier 3
    /// greedy coloring threshold. Values defined in these blocks are directed
    /// to Tier 2 (liveness-packed) instead of Tier 3 (greedy reuse).
    large_blocks: FxHashSet<usize>,
    /// Loop nesting depth per block index (0 = not inside any loop).
    ///
    /// Allocas whose single use-block is inside a loop MUST NOT be placed in
    /// the shared block-local slot region: that region reuses offsets across
    /// blocks under the assumption that blocks execute exclusively, but a
    /// loop body re-enters — so a loop-carried alloca (written in iteration N,
    /// read in iteration N+1, e.g. adler32's vs1/vs1_0 or a sum accumulator)
    /// is clobbered by another value allocated at the same offset
    /// (simd_avx2_256 mul check and vector_defer_multidef_slot regressions).
    block_loop_depth: Vec<u32>,
}

// ── Main stack space calculation ──────────────────────────────────────────

/// Shared stack space calculation: iterates over all instructions, assigns stack
/// slots for allocas and value results. Arch-specific offset direction is handled
/// by the `assign_slot` closure.
///
/// `initial_offset`: starting offset (e.g., 0 for x86, 16 for ARM/RISC-V to skip saved regs)
/// `assign_slot`: maps (current_space, raw_alloca_size, alignment) -> (slot_offset, new_space)
pub fn calculate_stack_space_common(
    state: &mut super::state::CodegenState,
    func: &IrFunction,
    initial_offset: i64,
    assign_slot: impl Fn(i64, i64, i64) -> (i64, i64),
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
    cached_liveness: Option<super::liveness::LivenessResult>,
) -> i64 {
    let num_blocks = func.blocks.len();

    // Enable coalescing and multi-tier allocation for any multi-block function.
    // Even small functions benefit: a 3-block function with 20 intermediates can
    // save 100+ bytes. Critical for recursive functions (PostgreSQL plpgsql) and
    // kernel functions with macro-expanded short-lived intermediates.
    let coalesce = num_blocks >= 2 && std::env::var("CCC_NO_SLOT_COALESCE").is_err();

    // Phase 0: Pre-scan for vector intrinsics and mark their destinations.
    // This must happen BEFORE stack layout because the slot allocator needs to
    // know which values are vectors in order to protect them from slot reuse.
    // Vector values need protected slots because reduction vectorization creates
    // tight dependency chains (init_zero → vec_load → vec_add → PHI) where
    // premature slot reuse corrupts intermediate vector data.
    use crate::ir::intrinsics::IntrinsicOp;
    let debug_protect = std::env::var("LCCC_DEBUG_PROTECT").is_ok();
    if debug_protect {
        eprintln!(
            "[PROTECT] Scanning function {} for vector intrinsics",
            func.name
        );
    }

    // Pass 1: Mark all vector intrinsic destinations as vector values.
    // Uses the single authority `IntrinsicOp::vector_result_width()` so that
    // USER-level SSE/AVX intrinsics (sad_epu8, maddubs, add_epi32, mul_ps,
    // loadu_si256, ...) are protected exactly like the auto-vectorizer's
    // internal Vec* ops. Previously only the Vec* ops were recognized, so
    // real SIMD code (zlib-ng adler32_ssse3, AVX2 intrinsic chains) got
    // 8-byte slots that 16/32-byte stores overflowed and that block-local
    // reuse reassigned while still live (vector_defer_multidef_slot,
    // simd_avx2_256 regressions).
    for block in &func.blocks {
        for inst in &block.instructions {
            if let crate::ir::instruction::Instruction::Intrinsic {
                dest: Some(d), op, ..
            } = inst
            {
                match op.vector_result_width() {
                    Some(32) => {
                        state.vector_values.insert(d.0);
                        state.protected_slot_values.insert(d.0);
                        if debug_protect {
                            eprintln!(
                                "[PROTECT] Marked SSA value {} as protected 256-bit ({:?})",
                                d.0, op
                            );
                        }
                    }
                    Some(16) => {
                        state.vector128_values.insert(d.0);
                        state.vector_values.insert(d.0);
                        state.protected_slot_values.insert(d.0);
                        if debug_protect {
                            eprintln!(
                                "[PROTECT] Marked SSA value {} as protected 128-bit ({:?})",
                                d.0, op
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Pass 2: Mark PHI nodes that have vector incoming values as vector/protected.
    // This handles the vectorized accumulator PHI: %acc_phi = phi [%init_zero, entry], [%vec_add, latch]
    // Must be done AFTER Pass 1 so that incoming vector values are already marked.
    // NOTE: PHI nodes are eliminated before stack layout, so this won't find them.
    // Instead, we rely on Pass 3 (Copy propagation) to mark the Copy instructions
    // that replace PHI nodes.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let crate::ir::instruction::Instruction::Phi { dest, incoming, .. } = inst {
                if debug_protect {
                    eprintln!(
                        "[PROTECT-PHI] Checking PHI SSA {}, incoming: {:?}",
                        dest.0, incoming
                    );
                }
                // If any incoming value is a vector, the PHI result is also a vector
                let has_vector_incoming = incoming.iter().any(|(val, _)| {
                    if let crate::ir::instruction::Operand::Value(v) = val {
                        let is_vec = state.vector_values.contains(&v.0);
                        if debug_protect && is_vec {
                            eprintln!("[PROTECT-PHI]   Incoming SSA {} is a vector!", v.0);
                        }
                        is_vec
                    } else {
                        false
                    }
                });
                if has_vector_incoming {
                    state.vector_values.insert(dest.0);
                    // A 128-bit class propagates only when every vector incoming
                    // is 128-bit; any 256-bit incoming makes the PHI 256-bit.
                    let all_incoming_128 = incoming.iter().all(|(val, _)| {
                        if let crate::ir::instruction::Operand::Value(v) = val {
                            !state.vector_values.contains(&v.0)
                                || state.vector128_values.contains(&v.0)
                        } else {
                            true
                        }
                    });
                    if all_incoming_128 {
                        state.vector128_values.insert(dest.0);
                    }
                    state.protected_slot_values.insert(dest.0);
                    if debug_protect {
                        eprintln!(
                            "[PROTECT] Marked PHI SSA value {} as protected (has vector incoming)",
                            dest.0
                        );
                    }
                }
            }
        }
    }

    // Pass 3: Propagate protection through Copy instructions.
    // PHI elimination converts `%acc = phi [%init, entry], [%result, latch]` into
    // Copy instructions. If the source of a Copy is protected, the dest should be too.
    // Iterate until no new protected values are found (fixed-point).
    loop {
        let initial_count = state.protected_slot_values.len();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let crate::ir::instruction::Instruction::Copy { dest, src } = inst {
                    if let crate::ir::instruction::Operand::Value(src_val) = src {
                        // Propagate only from genuinely VECTOR sources. Protected
                        // values also include scalar holders (DynAlloca results,
                        // addresses); marking those as vectors would switch their
                        // codegen from value-load to leaq addressing.
                        if state.vector_values.contains(&src_val.0) {
                            let was_new = state.protected_slot_values.insert(dest.0);
                            let was_vec = state.vector_values.insert(dest.0);
                            if state.vector128_values.contains(&src_val.0) {
                                state.vector128_values.insert(dest.0);
                            }
                            if debug_protect {
                                eprintln!("[PROTECT-COPY] Marked SSA {} as protected (copy from vector SSA {})", dest.0, src_val.0);
                            }
                        }
                    }
                }
            }
        }
        if state.protected_slot_values.len() == initial_count {
            break; // Fixed point reached
        }
    }

    if debug_protect {
        eprintln!(
            "[PROTECT] Total protected values: {}",
            state.protected_slot_values.len()
        );
    }

    // Phase 1: Build analysis context (use-blocks, def-blocks, used values,
    //          dead param allocas, alloca coalescability, copy aliases).
    let ctx = build_layout_context(
        func,
        coalesce,
        reg_assigned,
        callee_saved_regs,
        &state.ra_accumulator_values,
        &cached_liveness,
    );

    // Publish every explicit non-stack home through one location contract.
    state.explicit_locations.clear();
    for &id in &ctx.immediately_consumed {
        state
            .explicit_locations
            .insert(id, crate::backend::state::ExplicitLocation::Accumulator);
    }
    for (&id, &reg) in reg_assigned {
        // A real allocated register wins over the accumulator hint.
        state
            .explicit_locations
            .insert(id, crate::backend::state::ExplicitLocation::Reg(reg));
    }

    // vector-intrinsic results that can skip their slot store (their only
    // use is the adjacent intrinsic's args[0]/args[1] load).
    state.vector_defer_values = ctx.vector_defer_values.clone();
    state.vector_memfold_values = ctx.vector_memfold_values.clone();
    // i686 x87 top-of-stack deferral: F64 binop results consumed exactly once
    // by the adjacent F64 binop (the x87 twin of the vector deferral above;
    // backends that never set x87_pending ignore it).
    state.x87_defer_values = ctx.x87_defer_values.clone();
    // i686 x87 top-of-stack deferral: F64 binop results consumed exactly once
    // by the adjacent F64 binop (the x87 twin of the vector deferral above;
    // backends that never set x87_pending ignore it).

    // Phase 2: Classify all instructions into the three tiers.
    let mut non_local_space = initial_offset;
    let mut deferred_slots: Vec<DeferredSlot> = Vec::new();
    let mut multi_block_values: Vec<MultiBlockValue> = Vec::new();
    let mut block_local_values: Vec<BlockLocalValue> = Vec::new();
    let mut block_space: FxHashMap<usize, i64> = FxHashMap::default();
    let mut max_block_local_space: i64 = 0;

    slot_assignment::classify_instructions(
        state,
        func,
        &ctx,
        &assign_slot,
        reg_assigned,
        &mut non_local_space,
        &mut deferred_slots,
        &mut multi_block_values,
        &mut block_local_values,
        &mut block_space,
        &mut max_block_local_space,
    );

    // Phase 3: Tier 3 — block-local greedy slot reuse.
    slot_assignment::assign_tier3_block_local_slots(
        state,
        func,
        &ctx,
        coalesce,
        &block_local_values,
        &mut deferred_slots,
        &mut block_space,
        &mut max_block_local_space,
        &assign_slot,
    );

    // Raptor Lake Optimization: Protect cross-block vector slots
    for mbv in &multi_block_values {
        if state.vector_values.contains(&mbv.dest_id)
            || state.vector128_values.contains(&mbv.dest_id)
        {
            state.protected_slot_values.insert(mbv.dest_id);
        }
    }

    // Values participating in location rewrites are not ordinary SSA webs.
    // Copy aliases are resolved after Tier-2, multi-def values have several
    // emission stores, and phi incoming values are live on predecessor edges.
    // Keep them unique until the colorer models those edge/alias constraints.
    let mut tier2_location_sensitive = ctx.multi_def_values.clone();
    tier2_location_sensitive.extend(ctx.phi_incoming_values.iter().copied());
    for (&alias, &root) in &ctx.copy_alias {
        tier2_location_sensitive.insert(alias);
        tier2_location_sensitive.insert(root);
    }
    for mbv in &multi_block_values {
        if tier2_location_sensitive.contains(&mbv.dest_id)
            || state.asm_output_values.contains(&mbv.dest_id)
        {
            state.protected_slot_values.insert(mbv.dest_id);
        }
    }

    // Phase 4: Tier 2 — liveness-based packing for multi-block values.
    slot_assignment::assign_tier2_liveness_packed_slots(
        state,
        coalesce,
        cached_liveness,
        func,
        &multi_block_values,
        &mut non_local_space,
        &assign_slot,
    );

    // Phase 5: Finalize deferred block-local slots by adding the global base offset.
    let total_space = slot_assignment::finalize_deferred_slots(
        state,
        &deferred_slots,
        non_local_space,
        max_block_local_space,
        &assign_slot,
    );

    // Phase 6: Resolve copy aliases (propagate slots from root to aliased values).
    // Uses liveness information to avoid sharing slots between values with
    // overlapping lifetimes (which would corrupt one of the values).
    slot_assignment::resolve_copy_aliases(
        state,
        &ctx.copy_alias,
        &ctx.phi_web_aliases,
        &ctx.loop_phi_aliases,
        func,
    );

    // Phase 7: Propagate wide-value status through Copy chains (32-bit targets only).
    slot_assignment::propagate_wide_values(state, func, &ctx.copy_alias);

    // Diagnostic: dump the full value→slot map (and the vector/defer sets) so
    // slot aliasing miscompiles can be root-caused from the layout alone.
    // Usage: CCC_DEBUG_SLOTS=1 lccc -O2 -c file.c
    if std::env::var("CCC_DEBUG_SLOTS").is_ok() {
        eprintln!("[SLOTS] fn={} total_space={}", func.name, total_space);
        let mut ids: Vec<u32> = state.value_locations.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let slot = state.value_locations[&id].0;
            let tags: Vec<&str> = {
                let mut t = Vec::new();
                if state.vector_values.contains(&id) {
                    t.push(if state.vector128_values.contains(&id) {
                        "v128"
                    } else {
                        "v256"
                    });
                }
                if state.protected_slot_values.contains(&id) {
                    t.push("prot");
                }
                if state.vector_defer_values.contains(&id) {
                    t.push("defer");
                }
                if state.alloca_values.contains(&id) {
                    t.push("alloca");
                }
                t
            };
            eprintln!("[SLOTS]   v{:<5} slot={:<5} {}", id, slot, tags.join(","));
        }
    }

    total_space
}

// ── Phase 1: Build analysis context ───────────────────────────────────────

/// Build all the analysis data needed by the three-tier slot allocator.
/// This includes use-block maps, definition tracking, copy coalescing analysis,
/// dead param detection, and alloca coalescability.
fn build_layout_context(
    func: &IrFunction,
    coalesce: bool,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
    accumulator_values: &FxHashSet<u32>,
    cached_liveness: &Option<super::liveness::LivenessResult>,
) -> StackLayoutContext {
    // Build use-block map
    let mut use_blocks_map = if coalesce {
        analysis::compute_value_use_blocks(func)
    } else {
        FxHashMap::default()
    };

    // Build def-block map and identify multi-definition values (phi elimination).
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    let mut multi_def_values: FxHashSet<u32> = FxHashSet::default();
    if coalesce {
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    if let Some(&prev_blk) = def_block.get(&dest.0) {
                        if prev_blk != block_idx {
                            multi_def_values.insert(dest.0);
                        }
                    }
                    def_block.insert(dest.0, block_idx);
                }
            }
        }
    }

    // Collect all Value IDs referenced as operands (for dead value/param detection).
    let used_values = analysis::collect_used_values(func);

    // Detect dead parameter allocas.
    let dead_param_allocas =
        analysis::find_dead_param_allocas(func, &used_values, reg_assigned, callee_saved_regs);

    // Authoritative ABI alignment of struct/union parameter allocas (the
    // Alloca.align may have been dropped by a pass; IrParam.struct_align is not).
    // `param_alloca_values` and `params` are index-aligned (both include the
    // sret pointer at index 0 when present).
    let param_aligns: FxHashMap<u32, usize> = func
        .params
        .iter()
        .zip(func.param_alloca_values.iter())
        .filter_map(|(p, v)| p.struct_align.map(|a| (v.0, a)))
        .collect();

    // Alloca coalescability analysis.
    let coalescable_allocas = if coalesce {
        alloca_coalescing::compute_coalescable_allocas(
            func,
            &dead_param_allocas,
            &func.param_alloca_values,
        )
    } else {
        CoalescableAllocas {
            single_block: FxHashMap::default(),
            dead: FxHashSet::default(),
        }
    };

    // Copy coalescing analysis.
    let (copy_alias, phi_web_aliases, loop_phi_aliases) = copy_coalescing::build_copy_alias_map(
        func,
        &def_block,
        &multi_def_values,
        reg_assigned,
        &use_blocks_map,
        cached_liveness,
    );

    // Accumulator homes are selected and verified by register allocation.
    let immediately_consumed = accumulator_values.clone();

    // Deferred-store analysis: vector-intrinsic results whose single use is
    // args[0]/args[1] of the immediately-following intrinsic (accumulator renaming).
    if std::env::var("CCC_DEBUG_VDEFER").is_ok() {
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                match inst {
                    crate::ir::instruction::Instruction::Intrinsic {
                        op, dest_ptr, args, ..
                    } => {
                        let argids: Vec<String> = args
                            .iter()
                            .map(|a| match a {
                                crate::ir::instruction::Operand::Value(v) => v.0.to_string(),
                                _ => "_".to_string(),
                            })
                            .collect();
                        eprintln!(
                            "[VDEFER-IR] b{} i{} Intrinsic {:?} dp={:?} args=[{}]",
                            bi,
                            ii,
                            op,
                            dest_ptr.map(|v| v.0),
                            argids.join(",")
                        );
                    }
                    crate::ir::instruction::Instruction::Memcpy { dest, src, size } => {
                        eprintln!(
                            "[VDEFER-IR] b{} i{} Memcpy dest={} src={} size={}",
                            bi, ii, dest.0, src.0, size
                        );
                    }
                    crate::ir::instruction::Instruction::Copy { src, dest, .. } => {
                        eprintln!(
                            "[VDEFER-IR] b{} i{} Copy dest={} src={:?}",
                            bi, ii, dest.0, src
                        );
                    }
                    other => {
                        eprintln!(
                            "[VDEFER-IR] b{} i{} {:?}",
                            bi,
                            ii,
                            std::mem::discriminant(other)
                        );
                    }
                }
            }
        }
    }
    let vector_defer_values = copy_coalescing::compute_vector_defer_values(func);
    let vector_memfold_values = copy_coalescing::compute_vector_memfold_values(func);
    let x87_defer_values = copy_coalescing::compute_x87_defer_values(func);

    // Propagate copy-alias uses into use_blocks_map so that root values account
    // for their aliases' use sites when deciding block-local vs. multi-block.
    if coalesce && !copy_alias.is_empty() {
        for (&dest_id, &root_id) in &copy_alias {
            if let Some(dest_blocks) = use_blocks_map.get(&dest_id).cloned() {
                let root_blocks = use_blocks_map.entry(root_id).or_insert_with(Vec::new);
                for blk in dest_blocks {
                    if root_blocks.last() != Some(&blk) {
                        root_blocks.push(blk);
                    }
                }
            }
        }
    }

    // F128 load pointer promotion: when an F128 Load uses a non-alloca pointer,
    // the codegen records that pointer as the reload source for the full-precision
    // 128-bit value. If the loaded F128 dest is used in other blocks, the pointer
    // must remain accessible during those blocks' codegen. Without this, the
    // pointer stays block-local (Tier 3) and its slot gets reused by other
    // blocks' local values, causing the F128 reload to dereference garbage.
    //
    // Fix: propagate the F128 dest's use-blocks into the pointer's use-blocks,
    // forcing the pointer to Tier 2 (multi-block) when the dest crosses blocks.
    if coalesce {
        // Collect alloca value IDs to distinguish direct vs. indirect sources.
        let alloca_set: FxHashSet<u32> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| {
                if let Instruction::Alloca { dest, .. } = inst {
                    Some(dest.0)
                } else {
                    None
                }
            })
            .collect();

        // Collect (ptr_id, dest_id) pairs for F128 loads from non-alloca pointers.
        let f128_loads: Vec<(u32, u32)> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| {
                if let Instruction::Load { dest, ptr, ty, .. } = inst {
                    if *ty == IrType::F128 && !alloca_set.contains(&ptr.0) {
                        return Some((ptr.0, dest.0));
                    }
                }
                None
            })
            .collect();

        for (ptr_id, dest_id) in f128_loads {
            if let Some(dest_blocks) = use_blocks_map.get(&dest_id).cloned() {
                let ptr_blocks = use_blocks_map.entry(ptr_id).or_insert_with(Vec::new);
                for blk in dest_blocks {
                    if !ptr_blocks.contains(&blk) {
                        ptr_blocks.push(blk);
                    }
                }
            }
        }
    }

    // Collect Memcpy dest/src value widths: struct copies must get slots at
    // least as wide as the copy, otherwise the 16/32-byte copy overflows the
    // 8-byte fallback slot into neighbours.
    let mut memcpy_value_sizes: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Memcpy { dest, src, size } = inst {
                let e = memcpy_value_sizes.entry(dest.0).or_insert(0);
                if *size > *e {
                    *e = *size;
                }
                let e2 = memcpy_value_sizes.entry(src.0).or_insert(0);
                if *size > *e2 {
                    *e2 = *size;
                }
            }
        }
    }

    // Collect phi incoming values: values used as operands in Phi instructions.
    let mut phi_incoming_values = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let crate::ir::reexports::Instruction::Phi { incoming, .. } = inst {
                for (op, _) in incoming {
                    if let crate::ir::reexports::Operand::Value(v) = op {
                        phi_incoming_values.insert(v.0);
                    }
                }
            }
        }
    }

    // Loop depth per block: allocas used only inside a loop must stay out of
    // the shared block-local region (loop-carried lifetimes alias across
    // iterations). If liveness was not computed, conservatively treat every
    // block as looped (disables block-local allocas entirely).
    let block_loop_depth = cached_liveness
        .as_ref()
        .map(|l| l.block_loop_depth.clone())
        .unwrap_or_else(|| vec![1u32; func.blocks.len()]);

    // Identify blocks too large for Tier 3 greedy coloring.
    // The accumulator-based codegen processes instructions sequentially, but
    // the actual spill ordering can diverge from IR instruction order in
    // large blocks (register cache effects, multi-operand instructions,
    // fused operations). Tier 2 liveness-packed allocation uses proper live
    // interval computation that handles these cases correctly.
    let large_blocks: FxHashSet<usize> = func
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.instructions.len() > slot_assignment::MAX_TIER3_BLOCK_INSTRUCTIONS)
        .map(|(i, _)| i)
        .collect();

    StackLayoutContext {
        coalesce,
        use_blocks_map,
        def_block,
        multi_def_values,
        copy_alias,
        phi_web_aliases,
        loop_phi_aliases,
        used_values,
        dead_param_allocas,
        param_aligns,
        coalescable_allocas,
        immediately_consumed,
        vector_defer_values,
        vector_memfold_values,
        x87_defer_values,
        phi_incoming_values,
        memcpy_value_sizes,
        large_blocks,
        block_loop_depth,
    }
}
