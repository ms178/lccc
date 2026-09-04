//! Optimization passes for the IR.
//!
//! This module contains various optimization passes that transform the IR
//! to produce better code.
//!
//! Optimization levels are distinct but intentionally conservative:
//! - O0: no optimization pipeline (only inline-asm symbol resolution)
//! - O1: mem2reg + constant folding + copy propagation + DCE
//! - O2: the default full pipeline
//! - O3: O2 plus loop unrolling
//! - Os: O2 with code-size-increasing transforms disabled
//! - Oz: Os with inlining disabled

pub(crate) mod aggregate_copy_forward;
pub(crate) mod aggregate_sroa;
pub(crate) mod alias;
pub(crate) mod backedge_pre;
pub(crate) mod bit_idioms;
pub(crate) mod cvp;
pub(crate) mod block_layout;
pub(crate) mod bool_thread;
pub(crate) mod cfg_simplify;
pub(crate) mod constant_fold;
pub(crate) mod copy_prop;
pub(crate) mod dce;
mod dead_statics;
pub(crate) mod div_by_const;
pub(crate) mod dse;
pub(crate) mod fortify_fold;
pub(crate) mod fp_const_hoist;
pub(crate) mod global_addr_cse;
pub(crate) mod gvn;
pub(crate) mod if_convert;
pub(crate) mod inline;
pub(crate) mod instrument;
pub(crate) mod int_const_hoist;
pub(crate) mod ip_purity;
pub(crate) mod ipcp;
pub(crate) mod iv_strength_reduce;
pub(crate) mod iv_widen;
pub(crate) mod licm;
pub(crate) mod load_forward;
pub(crate) mod loop_analysis;
pub(crate) mod loop_invert;
pub(crate) mod loop_carried_forward;
pub(crate) mod loop_memory_promote;
pub(crate) mod loop_rotate;
pub(crate) mod loop_unroll;
pub(crate) mod narrow;
pub(crate) mod outline_switch;
pub(crate) mod quadratic_sr;
pub(crate) mod range_check;
pub(crate) mod reassoc_accum;
pub(crate) mod recursion_to_iter;
pub(crate) mod redundant_loads;
mod resolve_asm;
pub(crate) mod sccp;
pub(crate) mod set_membership;
pub(crate) mod simplify;
pub(crate) mod store_load_forward;
pub(crate) mod tail_call_elim;
pub(crate) mod univsr;
pub(crate) mod use_def;
pub(crate) mod vec_load_sink;
pub(crate) mod vector_temp_promotion;
pub(crate) mod vectorize;
pub(crate) mod verify;

use crate::common::fp_contract::FpContract;
use crate::common::fx_hash::FxHashSet;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{BasicBlock, Instruction, IrFunction, IrModule, Operand};

/// CCC_VALIDATE_SSA=1 debug validator: every Value id must have exactly one
/// defining instruction, InlineAsm outputs and Phi dests included, and every
/// def must be strictly below the function's `next_value_id` watermark.
///
/// A stale watermark is the *origin* of duplicate-def bugs: the offending
/// pass clones or synthesizes instructions with ids at/above `next_value_id`,
/// and a later pass that allocates "fresh" ids from the watermark silently
/// collides with them. The RA then computes one merged live range for two
/// unrelated values and the backend's no-home hard gate fires far from the
/// cause (glibc __libc_start_main_impl: Cast and GetElementPtr both defining
/// v356). Panics at the FIRST violating phase so the bisection is exact.
///
/// `strict`: before phi elimination the IR is SSA and every duplicate def is
/// a violation. After `eliminate_phis` the IR is *conventional* SSA — a phi
/// home is legally assigned by one `Copy` per predecessor edge — so
/// duplicates are tolerated iff every def of that id is a `Copy`.
pub(crate) fn validate_unique_defs(module: &IrModule, tag: &str) {
    let strict = !tag.starts_with("backend:eliminate_phis")
        && !tag.starts_with("backend:post-phi")
        && !tag.starts_with("backend:pre-codegen");
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        let mut def_of: crate::common::fx_hash::FxHashMap<u32, (String, bool)> =
            crate::common::fx_hash::FxHashMap::default();
        let mut max_def: u32 = 0;
        let mut check = |id: u32, what: String, is_copy: bool| {
            max_def = max_def.max(id);
            if let Some((prev, prev_copy)) = def_of.insert(id, (what.clone(), is_copy)) {
                if strict || !(is_copy && prev_copy) {
                    panic!(
                        "SSA VIOLATION after phase '{}': value v{} in function '{}' \
                         defined twice:\n  first : {}\n  second: {}",
                        tag, id, func.name, prev, what
                    );
                }
            }
        };
        for (bi, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    let is_copy = matches!(inst, Instruction::Copy { .. });
                    check(d.0, format!("block {} {:?}", bi, inst), is_copy);
                }
                // NOTE: InlineAsm `outputs` carry value_POINTERS (addresses
                // the asm stores through) — they are uses, not defs, and may
                // legitimately repeat a GEP dest. They are deliberately not
                // checked here.
            }
        }
        if max_def >= func.next_value_id {
            panic!(
                "SSA WATERMARK VIOLATION after phase '{}': function '{}' has \
                 def v{} >= next_value_id {} — the phase that created it did \
                 not bump the watermark; later passes will allocate colliding \
                 'fresh' ids",
                tag, func.name, max_def, func.next_value_id
            );
        }

        // PHI-INCOMING DOMINANCE: an incoming (v, pred) is evaluated at the
        // END OF pred, so v must be defined in a block dominating pred. The
        // case passes actually get wrong: substituting a SIBLING phi's dest
        // as an incoming on a FORWARD edge (value equality reasoning that is
        // only true under sequential-copy semantics). On a backedge the phi
        // block dominates the latch and sibling-dest incomings are the
        // classic legal swap-loop shape. glibc _dl_lookup_symbol_x: v445's
        // entry-edge incoming was rewritten v8 -> v454 (sibling phi),
        // eliminate_phis then emitted `v445 = Copy(v454)` before v454's own
        // edge copy — every ld.so symbol lookup hashed a stale register
        // (LK-27).
        if strict {
            let cfg = crate::ir::analysis::CfgAnalysis::build(func);
            let dominates = |a: usize, b: usize| -> bool {
                let mut x = b;
                loop {
                    if x == a {
                        return true;
                    }
                    let n = cfg.idom[x];
                    if n == x {
                        return x == a;
                    }
                    x = n;
                }
            };
            let mut label_of: crate::common::fx_hash::FxHashMap<u32, usize> =
                crate::common::fx_hash::FxHashMap::default();
            for (bi, block) in func.blocks.iter().enumerate() {
                label_of.insert(block.label.0, bi);
            }
            let mut def_block: crate::common::fx_hash::FxHashMap<u32, usize> =
                crate::common::fx_hash::FxHashMap::default();
            for (bi, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    if let Some(d) = inst.dest() {
                        def_block.insert(d.0, bi);
                    }
                }
            }
            for (bi, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        for (op, pred) in incoming {
                            if let Operand::Value(v) = op {
                                if let (Some(&db), Some(&pb)) =
                                    (def_block.get(&v.0), label_of.get(&pred.0))
                                {
                                    if !dominates(db, pb) {
                                        panic!(
                                            "SSA PHI-DOMINANCE VIOLATION after phase '{}': \
                                             function '{}' block {} phi v{} incoming v{} \
                                             from pred block {} is not dominated by the \
                                             defining block {}",
                                            tag, func.name, bi, dest.0, v.0, pb, db
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // SAME-BLOCK ORDER: a use of v inside the block that defines v must
        // come AFTER the definition. Uniqueness alone does not catch a pass
        // that swaps two instructions (glibc _dl_lookup_symbol_x: the
        // `s = undef_name` copy was emitted before undef_name's own home
        // copy, so the inlined _dl_new_hash hashed a stale register and
        // every symbol lookup against ld.so's map failed — LK-27). Cross-
        // block dominance is deliberately not modelled here; the same-block
        // case is the one instruction-motion bugs actually produce, and it
        // needs no CFG analysis. Skipped in the conventional-SSA backend
        // phases where phi-home Copies legitimately repeat.
        // In conventional-SSA backend phases, cross-block flow can make a
        // same-block late def legal (loop-rotated phi homes) — but the ENTRY
        // block has no predecessors: use-before-def there is a bug in every
        // phase. Check all blocks in strict phases, entry-only afterwards.
        let order_blocks = if strict { usize::MAX } else { 1 };
        {
            for (bi, block) in func.blocks.iter().enumerate().take(order_blocks) {
                let mut defined_here: crate::common::fx_hash::FxHashMap<u32, usize> =
                    crate::common::fx_hash::FxHashMap::default();
                for (ii, inst) in block.instructions.iter().enumerate() {
                    if let Some(d) = inst.dest() {
                        defined_here.entry(d.0).or_insert(ii);
                    }
                }
                for (ii, inst) in block.instructions.iter().enumerate() {
                    // Phi incoming values are PARALLEL COPIES on predecessor
                    // EDGES: a loop-carried value is legally defined later in
                    // the same (header) block, arriving via the backedge.
                    // Order only constrains straight-line uses.
                    if matches!(inst, Instruction::Phi { .. }) {
                        continue;
                    }
                    let mut bad: Option<u32> = None;
                    inst.for_each_used_value(|v| {
                        if bad.is_none() {
                            if let Some(&di) = defined_here.get(&v) {
                                if di > ii {
                                    bad = Some(v);
                                }
                            }
                        }
                    });
                    if let Some(v) = bad {
                        panic!(
                            "SSA ORDER VIOLATION after phase '{}': function '{}' \
                             block {} instruction {} uses v{} which is only \
                             defined later in the same block (index {}):\n  use: {:?}",
                            tag, func.name, bi, ii, v, defined_here[&v], inst
                        );
                    }
                }
            }
        }
    }
}

/// Debug hook: dump only functions whose name contains one of the comma-
/// separated substrings in `CCC_DUMP_FUNC` (empty = dump all), in the same
/// compact per-block format as the backend's CCC_DUMP_IR.
/// Environment-gated; never changes codegen.
fn dump_ir_filtered(module: &IrModule, tag: &str) {
    if std::env::var_os("CCC_VALIDATE_SSA").is_some() {
        validate_unique_defs(module, tag);
    }
    if std::env::var_os("CCC_DUMP_EACH_PASS").is_none() {
        return;
    }
    use std::fmt::Write as _;
    let filters: Vec<String> = std::env::var("CCC_DUMP_FUNC")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "==== IR {} ====", tag);
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        let matched = filters.is_empty() || filters.iter().any(|f| func.name.contains(f.as_str()));
        if matched {
            let _ = writeln!(
                out,
                "--- function {} ({} blocks, {} instrs) ---",
                func.name,
                func.blocks.len(),
                func.blocks
                    .iter()
                    .map(|b| b.instructions.len())
                    .sum::<usize>()
            );
            for (bi, block) in func.blocks.iter().enumerate() {
                let _ = writeln!(out, "block {} ({}):", bi, block.label);
                for inst in &block.instructions {
                    let _ = writeln!(out, "  {:?}", inst);
                }
                let _ = writeln!(out, "  term: {:?}", block.terminator);
            }
        }
    }
    let _ = writeln!(out, "==== END IR {} ====", tag);
    eprintln!("{}", out);
}

/// Run a per-function pass only on functions in the visit set.
///
/// `visit` indicates which functions to process in this iteration.
/// `changed` accumulates which functions were modified by any pass
/// (so the next iteration knows what to re-visit).
fn run_on_visited<F>(module: &mut IrModule, visit: &[bool], changed: &mut [bool], mut f: F) -> usize
where
    F: FnMut(&mut IrFunction) -> usize,
{
    let mut total = 0;
    for (i, func) in module.functions.iter_mut().enumerate() {
        if func.is_declaration {
            continue;
        }
        if i < visit.len() && !visit[i] {
            continue;
        }
        let n = f(func);
        if n > 0 {
            if i < changed.len() {
                changed[i] = true;
            }
            total += n;
        }
    }
    total
}

/// Run GVN, LICM, and IVSR with shared CFG analysis per function.
///
/// For each dirty function, builds CFG/dominator/loop analysis once and passes
/// it to all three passes. This eliminates redundant analysis computation that
/// previously occurred when each pass independently computed build_label_map +
/// build_cfg + compute_dominators (+ find_natural_loops for LICM/IVSR).
///
/// Returns (gvn_changes, licm_changes, ivsr_changes).
fn run_gvn_licm_ivsr_shared(
    module: &mut IrModule,
    visit: &[bool],
    changed: &mut [bool],
    run_gvn: bool,
    run_licm: bool,
    run_ivsr: bool,
    run_univsr: bool,
    time_passes: bool,
    iter: usize,
) -> (usize, usize, usize) {
    let mut gvn_total = 0usize;
    let mut licm_total = 0usize;
    let mut ivsr_total = 0usize;
    let gvn_context = gvn::GvnContext::for_module(module);

    for (i, func) in module.functions.iter_mut().enumerate() {
        if func.is_declaration {
            continue;
        }
        if i < visit.len() && !visit[i] {
            continue;
        }
        let num_blocks = func.blocks.len();
        if num_blocks == 0 {
            continue;
        }

        // GVN fast path: single-block functions don't need CFG analysis.
        if num_blocks == 1 {
            if run_gvn {
                let n = gvn::run_gvn_function_with_context(func, &gvn_context);
                verify::verify_after_func_pass(func, "gvn");
                if n > 0 {
                    gvn_total += n;
                    if i < changed.len() {
                        changed[i] = true;
                    }
                }
            }
            // LICM and IVSR need loops (>= 2 blocks), so skip.
            continue;
        }

        // Build CFG analysis once for this function.
        let cfg = CfgAnalysis::build(func);

        // Run GVN with shared analysis.
        if run_gvn {
            let t0 = if time_passes {
                Some(std::time::Instant::now())
            } else {
                None
            };
            let n = gvn::run_gvn_with_analysis_and_context(func, &cfg, &gvn_context);
            verify::verify_after_func_pass(func, "gvn");
            if let Some(t0) = t0 {
                eprintln!(
                    "[PASS] iter={} gvn (func {}): {:.4}s ({} changes)",
                    iter,
                    func.name,
                    t0.elapsed().as_secs_f64(),
                    n
                );
            }
            if n > 0 {
                gvn_total += n;
                if i < changed.len() {
                    changed[i] = true;
                }
            }
        }

        // Run LICM with shared analysis.
        // GVN does not modify the CFG (only replaces operands), so analysis is still valid.
        if run_licm {
            let t0 = if time_passes {
                Some(std::time::Instant::now())
            } else {
                None
            };
            let n = licm::licm_with_analysis(func, &cfg);
            verify::verify_after_func_pass(func, "licm");
            if let Some(t0) = t0 {
                eprintln!(
                    "[PASS] iter={} licm (func {}): {:.4}s ({} changes)",
                    iter,
                    func.name,
                    t0.elapsed().as_secs_f64(),
                    n
                );
            }
            if n > 0 {
                licm_total += n;
                if i < changed.len() {
                    changed[i] = true;
                }
            }
        }

        // Run IVSR with shared analysis.
        // LICM hoists instructions to preheaders but does not add/remove blocks,
        // so CFG analysis is still valid.
        // W4 (2026-08-10): reduce_loop gates every candidate GEP on
        // is_loop_invariant(base) — loop-VARIANT bases (the historical matmul
        // address-doubling bug) are skipped, so the pointer recurrence can
        // never compound with a moving base. Full battery verified with the
        // pass forced ON. The overlap guard now excludes nested-loop cases
        // that previously miscompiled matrix-style code, and the safe pass
        // reduces redundant induction casts in disjoint array loops. Keep
        // CCC_NO_IVSR=1 as a targeted diagnostic escape hatch.
        if run_ivsr {
            // One transformation can expose another independent pointer IV in
            // the same loop (notably the two source/destination GEPs created
            // by vectorization). Iterate locally; IVSR changes instructions
            // and phis but not CFG edges, so the shared analysis stays valid.
            let mut n = 0;
            for _ in 0..4 {
                let round = iv_strength_reduce::ivsr_with_analysis(func, &cfg);
                verify::verify_after_func_pass(func, "ivsr");
                n += round;
                if round == 0 {
                    break;
                }
            }
            if n > 0 {
                ivsr_total += n;
                if i < changed.len() {
                    changed[i] = true;
                }
            }
        }

        // Un-IVSR (x86-64 only): revert IVSR pointer IVs to indexed form so
        // the backend can use SIB addressing (base + index*scale + disp).
        // Runs directly after IVSR while the CFG analysis is still valid;
        // the pass rewrites instructions in place and never edits the CFG.
        if run_univsr {
            let n = univsr::run_univsr(func);
            verify::verify_after_func_pass(func, "univsr");
            if n > 0 {
                ivsr_total += n;
                if i < changed.len() {
                    changed[i] = true;
                }
                if time_passes {
                    eprintln!(
                        "[PASS] iter={} univsr (func {}): {} pointer IVs reverted",
                        iter, func.name, n
                    );
                }
            }
        }
    }

    if time_passes {
        eprintln!("[PASS] iter={} gvn_total: {} changes", iter, gvn_total);
        eprintln!("[PASS] iter={} licm_total: {} changes", iter, licm_total);
        eprintln!("[PASS] iter={} ivsr_total: {} changes", iter, ivsr_total);
    }

    (gvn_total, licm_total, ivsr_total)
}

/// Run all optimization passes on the module.
///
/// The pass pipeline is:
/// 1. CFG simplification (remove dead blocks, thread jump chains, simplify branches)
/// 2. Copy propagation (replace uses of copies with original values)
/// 3. Algebraic simplification (strength reduction)
/// 4. Constant folding (evaluate const exprs at compile time)
/// 5. GVN / CSE (dominator-based value numbering, eliminates redundant
///    BinOp, UnaryOp, Cmp, Cast, GetElementPtr, and Load across dominated blocks)
/// 6. LICM (hoist loop-invariant code to preheaders)
/// 7. If-conversion (convert branch+phi diamonds to Select)
/// 8. Copy propagation (clean up copies from GVN/simplify/LICM)
/// 9. Dead code elimination (remove dead instructions)
/// 10. CFG simplification (clean up after DCE may have made blocks dead)
/// 11. Dead static function elimination (remove unreferenced internal-linkage functions)
///
struct DisabledPasses {
    cfg: bool,
    copyprop: bool,
    narrow: bool,
    simplify: bool,
    constfold: bool,
    gvn: bool,
    licm: bool,
    ifconv: bool,
    dce: bool,
    dse: bool,
    ipcp: bool,
    unroll: bool,
}

impl DisabledPasses {
    fn from_env(disabled: &str) -> Self {
        DisabledPasses {
            cfg: pass_disabled(&disabled, "cfg"),
            copyprop: pass_disabled(&disabled, "copyprop"),
            narrow: pass_disabled(&disabled, "narrow"),
            simplify: pass_disabled(&disabled, "simplify"),
            constfold: pass_disabled(&disabled, "constfold"),
            gvn: pass_disabled(&disabled, "gvn"),
            licm: pass_disabled(&disabled, "licm"),
            ifconv: pass_disabled(&disabled, "ifconv"),
            dce: pass_disabled(&disabled, "dce"),
            dse: pass_disabled(&disabled, "dse"),
            ipcp: pass_disabled(&disabled, "ipcp"),
            unroll: pass_disabled(&disabled, "unroll"),
        }
    }
}

/// Restore the invariant that every phi forms a contiguous prefix of its block.
///
/// Several passes rewrite a phi into a non-phi *in place* — `univsr` turns a
/// reverted pointer-IV phi into a `Copy`, `cfg_simplify` collapses a
/// single-predecessor phi the same way. When the rewritten phi is not the last
/// one in the block, every phi after it is suddenly preceded by a non-phi, and
/// passes that assume contiguity (`loop_rotate`, `mem2reg`, phi elimination)
/// silently read the wrong instruction range.
///
/// Sinking the non-phis below the phi prefix is always sound: phis are defined
/// to execute simultaneously on entry to the block, so no phi may read a value
/// defined by a non-phi in that same block. (A phi's back-edge operand is
/// evaluated at the end of the *predecessor*, not here.) A stable partition
/// therefore preserves program semantics exactly while restoring the
/// invariant, and it keeps the relative order within each group so unrelated
/// diffs do not appear in dumps.
///
/// Returns `true` if anything moved. Runs in O(n) and allocates only when a
/// repair is actually needed, so callers can invoke it unconditionally.
pub(crate) fn restore_phi_prefix(block: &mut BasicBlock) -> bool {
    let insts = &block.instructions;
    // Fast path: find the first non-phi, then check nothing after it is a phi.
    let first_non_phi = match insts
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
    {
        Some(p) => p,
        None => return false, // all phis (or empty): already contiguous
    };
    if !insts[first_non_phi + 1..]
        .iter()
        .any(|i| matches!(i, Instruction::Phi { .. }))
    {
        return false; // already contiguous
    }

    // `source_spans` is parallel to `instructions`; permute it identically or
    // the backend emits .loc directives against the wrong instruction. Passes
    // have been observed to leave it desynchronised, so only touch it when the
    // lengths agree.
    let spans_valid = block.source_spans.len() == block.instructions.len();

    let n = block.instructions.len();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    order.extend((0..n).filter(|&i| matches!(block.instructions[i], Instruction::Phi { .. })));
    order.extend((0..n).filter(|&i| !matches!(block.instructions[i], Instruction::Phi { .. })));

    let mut old_insts = std::mem::take(&mut block.instructions);
    let mut taken: Vec<Option<Instruction>> = old_insts.drain(..).map(Some).collect();
    block.instructions = order
        .iter()
        .map(|&i| taken[i].take().expect("each index used once"))
        .collect();

    if spans_valid {
        let old_spans = std::mem::take(&mut block.source_spans);
        block.source_spans = order.iter().map(|&i| old_spans[i]).collect();
    }
    true
}

/// Apply [`restore_phi_prefix`] to every block of a function.
pub(crate) fn restore_phi_prefix_in_function(func: &mut IrFunction) -> usize {
    func.blocks
        .iter_mut()
        .map(|b| usize::from(restore_phi_prefix(b)))
        .sum()
}

/// Run Phase 0: function inlining and post-inline optimization passes.
fn run_inline_phase(module: &mut IrModule, disabled: &str, allow_inline: bool, size_profile: bool) {
    let dump_pre =
        std::env::var("CCC_DUMP_EACH_PASS").is_ok() || std::env::var("CCC_VALIDATE_SSA").is_ok();
    macro_rules! iphase_dump {
        ($name:expr) => {
            if dump_pre {
                dump_ir_filtered(module, &format!("pre-loop {}", $name));
            }
            verify::verify_after_pass(module, $name);
        };
    }
    // Canonicalize before cost analysis so inlining decisions use optimized
    // IR sizes (GVN shrinks helpers like expat's sip_round 229->84, making
    // them inlineable). GVN never rewrites param-alloca loads (see gvn.rs),
    // so the inliner's store-into-param-alloca argument passing stays intact.
    // Keep parameter allocas until after inlining.
    if !std::env::var("CCC_NO_MEM2REG").is_ok() {
        crate::ir::mem2reg::promote_allocas(module);
    }
    iphase_dump!("mem2reg");
    constant_fold::run(module);
    iphase_dump!("canonicalize-fold");
    copy_prop::run(module);
    iphase_dump!("canonicalize-copyprop");
    simplify::run(module);
    iphase_dump!("canonicalize-simplify");
    // Class-aware GlobalAddr CSE *before* the first GVN. GVN already CSEs
    // GlobalAddr into same-block Copies, so a late-only run after
    // post-structural-inline never sees duplicates.
    if !pass_disabled(&disabled, "gaddrcse") {
        global_addr_cse::run_module(module);
    }
    iphase_dump!("global_addr_cse-pre-gvn");
    if !pass_disabled(&disabled, "gvn") {
        // Use the module context (GNU alias canonicalization, global epoch
        // facts) exactly like the main pass loop: with the default context,
        // loads across stores to GNU-alias'd globals are CSE'd and produce
        // wrong code (regression gvn_global_symbol_alias).
        let gvn_ctx = gvn::GvnContext::for_module(module);
        module.for_each_function(|f| gvn::run_gvn_function_with_context(f, &gvn_ctx));
        module.for_each_function(dce::eliminate_dead_code);
    }
    constant_fold::run(module);
    copy_prop::run(module);

    if !pass_disabled(&disabled, "inline") {
        if size_profile {
            // -Os/-Oz: retain tiny/small and profitable one-use/loop
            // inlines, but reject repeated cold medium-body expansion.
            inline::run_size_optimized(module);
        } else if allow_inline {
            inline::run(module);
        }
    }
    if std::env::var("CCC_DUMP_IR").is_ok()
        || std::env::var("CCC_DUMP_EACH_PASS").is_ok()
        || std::env::var("CCC_VALIDATE_SSA").is_ok()
    {
        dump_ir_filtered(module, "pre-loop after inliner");
    }

    // After inlining, convert extern inline gnu_inline functions to declarations —
    // but ONLY when no calls remain. The bodies were only needed for inlining;
    // they must not be emitted as standalone definitions when fully inlined away,
    // because their internal calls (e.g., `call btowc`) would resolve to the local
    // definition instead of the external library symbol, causing infinite recursion.
    // However, if any call site was NOT inlined (e.g. the caller was skipped by the
    // inliner), the function MUST still be emitted as a local (static) definition —
    // GNU89 `extern inline` semantics provide an inline definition usable locally.
    // Dropping it here made glibc's rtld link fail with undefined references to
    // `free`, `__bsearch` etc.
    let mut still_called: FxHashSet<String> = FxHashSet::default();
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Call { func: callee, .. } = inst {
                    still_called.insert(callee.clone());
                }
            }
        }
    }
    for func in &mut module.functions {
        if func.is_gnu_inline_def && !func.is_declaration && !still_called.contains(&func.name) {
            func.is_declaration = true;
            func.blocks.clear();
        }
    }

    if !std::env::var("CCC_NO_MEM2REG_PARAMS").is_ok() {
        crate::ir::mem2reg::promote_allocas_with_params(module);
    }
    iphase_dump!("cleanup-mem2reg-params");
    if !std::env::var("CCC_NO_CLEANUP_FOLD1").is_ok() {
        constant_fold::run(module);
    }
    iphase_dump!("cleanup-fold1");
    if !std::env::var("CCC_NO_CLEANUP_CP1").is_ok() {
        copy_prop::run(module);
    }
    iphase_dump!("cleanup-copyprop1");
    // Copy forwarding can expose another temporary in a copy chain
    // (`object -> tmp1 -> tmp2 -> field load`), so run to a small fixed point.
    if !std::env::var("CCC_NO_AGG_COPY_FWD").is_ok() {
        for _ in 0..8 {
            if module.for_each_function(aggregate_copy_forward::run) == 0 {
                break;
            }
        }
    }
    iphase_dump!("cleanup-agg-copy-forward");
    // Promote loop-carried memory locations (struct fields / array cells with
    // invariant addresses) into SSA values across the loop body.
    if !std::env::var("CCC_NO_LOOP_MEM_PROMOTE").is_ok() {
        module.for_each_function(loop_memory_promote::run);
    }
    iphase_dump!("cleanup-loop-memory-promote");
    // Loop-carried store-to-load forwarding (distance-1 predictive
    // commoning): `a[i] = f(a[i-1])` keeps the recurrence in a register
    // instead of a store-forwarding round trip per iteration.
    loop_carried_forward::run_module(module);
    verify::verify_after_pass(module, "loop_carried_forward");
    iphase_dump!("cleanup-loop-carried-forward");
    // Resolve constant branches exposed by inlining before the bounded
    // constant-call evaluator in ipcp examines the surviving call sites.
    //
    // SCCP leads here because this is where it pays the most: inlining has
    // just substituted literal arguments into a callee body, so whole
    // parameter-dependent branches are now decidable. `cfg_simplify` can only
    // fold a branch whose condition is *already* a constant operand; SCCP
    // proves the condition constant in the first place, including through the
    // phis that inlining leaves at the callee's return join.
    if !pass_disabled(disabled, "sccp") {
        module.for_each_function(sccp::run_function);
    }
    iphase_dump!("cleanup-sccp");
    if !std::env::var("CCC_NO_CLEANUP_CFGSIMP").is_ok() {
        for _ in 0..3 {
            let n = module.for_each_function(cfg_simplify::run_function);
            constant_fold::run(module);
            copy_prop::run(module);
            if n == 0 {
                break;
            }
        }
    }
    iphase_dump!("cleanup-cfg-simplify");
    if !std::env::var("CCC_NO_CLEANUP_SIMP").is_ok() {
        simplify::run(module);
    }
    iphase_dump!("cleanup-simplify");
    if !std::env::var("CCC_NO_CLEANUP_FOLD2").is_ok() {
        constant_fold::run(module);
    }
    if !std::env::var("CCC_NO_CLEANUP_CP2").is_ok() {
        copy_prop::run(module);
    }
    iphase_dump!("cleanup-fold2-copyprop2");
    // Inlining + GEP+0 folding can turn `&local` stores back into stores
    // through the alloca itself. Re-run mem2reg so those scalars become SSA
    // before the main loop (otherwise they stay slotted across the callee
    // body that was just pasted in).
    if !std::env::var("CCC_NO_MEM2REG_PARAMS").is_ok() {
        crate::ir::mem2reg::promote_allocas_with_params(module);
    }
    iphase_dump!("cleanup-mem2reg-after-gep0");
    resolve_asm::resolve_inline_asm_symbols(module);
    iphase_dump!("post-inline cleanup");
}

/// Exact-token pass-disable matching for the comma-separated list shared by
/// `CCC_DISABLE_PASSES` and the m16 size policy. The previous substring test
/// let a longer pass name disable an unrelated shorter one: the m16 policy's
/// "postinline" entry silently disabled the primary "inline" phase (every
/// -m16 -Os/-Oz compile ran without ANY inlining — the exact corpus where
/// helper inlining matters most), and "univsr" likewise disabled "ivsr".
/// Tokens are compared trimmed and exactly; "all" keeps its global meaning.
fn pass_disabled(disabled: impl AsRef<str>, pass: &str) -> bool {
    disabled.as_ref().split(',').any(|tok| {
        let tok = tok.trim();
        tok == "all" || tok == pass
    })
}

fn apply_m16_size_policy(disabled: &mut String, code16gcc: bool, opt_level: u32) {
    if !code16gcc || opt_level < 4 {
        return;
    }
    // Measurement hook: CCC_M16_KEEP=<pass,...> removes individual passes
    // from the disable list so each can be A/B-measured in isolation. The
    // defaults are unchanged — this exists so the policy's entries stay
    // evidence-backed instead of cargo-culted from a conflated measurement.
    let keep: Vec<String> = std::env::var("CCC_M16_KEEP")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string())
        .collect();
    for pass in ["postinline", "ifconv", "gaddrcse", "licm"] {
        if keep.iter().any(|k| k == pass) {
            continue;
        }
        if !disabled.is_empty() {
            disabled.push(',');
        }
        disabled.push_str(pass);
    }
}

/// Run optimization passes for the requested optimization level.
///
/// `opt_level`: 0=-O0, 1=-O1, 2=-O2, 3=-O3, 4=-Os, 5=-Oz.
pub(crate) fn run_passes(
    module: &mut IrModule,
    opt_level: u32,
    target: crate::backend::Target,
    code16gcc: bool,
    fp_reassoc: bool,
    fp_contract: crate::common::fp_contract::FpContract,
    x86_avx: bool,
    x86_fma: bool,
) {
    // FMA3 ISA availability for the vectorizer's VecFma/VecMadd contraction
    // (see vectorize::set_x86_fma_enabled). AArch64 fmla is baseline ISA and
    // ignores this.
    vectorize::set_x86_fma_enabled(x86_fma && target == crate::backend::Target::X86_64);
    // FMA3 availability for the fma/fmaf libcall fold (simplify.rs): the
    // fused form is required for the C99 single-rounding semantics, so the
    // fold must only fire when the backend can actually emit vfmadd*.
    // x86-64's project baseline is x86-64-v3 (FMA3 always present); i686
    // has it only under -mfma.
    simplify::set_has_fma3(match target {
        crate::backend::Target::X86_64 => true,
        crate::backend::Target::I686 => x86_fma,
        crate::backend::Target::Aarch64 => true,
        _ => false,
    });
    let mut disabled = std::env::var("CCC_DISABLE_PASSES").unwrap_or_default();
    // Linux's `.code16gcc` setup image has a hard 32 KiB code+data+BSS limit
    // and only six generally usable GPRs.  On the real linux-cachymod setup
    // corpus these four transformations increase final machine-code size by
    // merging live ranges faster than they remove operations.  This is not a
    // generic i686 policy: ordinary -m32 keeps the throughput-oriented
    // pipeline.  Restrict the measured size profile to explicit -m16 -Os/-Oz.
    if std::env::var_os("CCC_M16_FULL_PIPELINE").is_none() {
        apply_m16_size_policy(&mut disabled, code16gcc, opt_level);
    }
    // Debug hook: dump the module IR to stderr before optimization.
    if std::env::var("CCC_DUMP_IR").is_ok() {
        eprintln!("==== IR before passes (opt_level={}) ====", opt_level);
        eprintln!("{:#?}", module);
        eprintln!("==== END IR ====");
    }
    if pass_disabled(&disabled, "all") {
        return;
    }

    let time_passes = std::env::var("CCC_TIME_PASSES").is_ok();
    // -fdump-tree-all equivalent: dump the module after every pass.
    let dump_each_pass =
        std::env::var("CCC_DUMP_EACH_PASS").is_ok() || std::env::var("CCC_VALIDATE_SSA").is_ok();

    // -O0: preserve the alloca-based IR and skip the optimizer completely.
    // Inline asm symbol resolution is not an optimization; it is required for
    // correct backend emission of asm operands.
    if opt_level == 0 {
        resolve_asm::resolve_inline_asm_symbols(module);
        // GCC semantics: always_inline is a correctness constraint, honored at
        // every optimization level.  gnu_inline wrappers (glibc
        // _FORTIFY_SOURCE, gcc.c-torture va-arg-pack-1.c) have no out-of-line
        // body, so an un-inlined always_inline call cannot link.  This tier
        // clones only always_inline callees; everything else is untouched.
        if !pass_disabled(&disabled, "inline") {
            inline::run_always_inline_only(module);
        }
        // _Float128 math builtins lower to libgcc helper calls (__copysigntf3,
        // __fabstf2); LCCC links no libgcc, so fold them to the backend's own
        // inline intrinsics even at -O0 (semantics-preserving rename).
        simplify::fold_math_intrinsic_calls(module);
        return;
    }

    // -O1: cheap scalar cleanup only.  This tier is intentionally small and
    // predictable for faster debug-style builds while still removing obvious
    // dead/copy/constant IR introduced by lowering.
    if opt_level == 1 {
        // Parameter allocas are promoted up front so the folding passes
        // below see ParamRef-SSA operands (range_fold + simplify need that
        // shape to fold 20041114-1's unsigned-range disjunction). The
        // reassociation single-use guard in simplify.rs removed the one
        // rewrite that mis-compiled this shape (va-arg-21's glibc
        // extern-inline vprintf body: the multi-use reassociation re-based a
        // def onto a different register web while the inlined va_list ABI
        // kept the old home).
        crate::ir::mem2reg::promote_allocas_with_params(module);
        if time_passes {
            eprintln!("[PASS] o1 mem2reg");
        }
        ip_purity::run(module);
        if !pass_disabled(&disabled, "fortifyfold") {
            fortify_fold::run(module, target.is_32bit());
        }
        constant_fold::run(module);
        copy_prop::run(module);
        // Local simplifier at -O1: shift identity folds (x>>0 -> x, 0<<x -> 0,
        // all-ones arithmetic shr), `fabs(x) < 0.0` -> false and related
        // per-instruction canonicalizations. gcc.c-torture shiftopt-1 and
        // 20020720-1 leave their link_error calls reachable without these
        // folds; GCC's front end / match.pd applies them at every level, so
        // -O1 parity requires them here. One linear per-function sweep keeps
        // the -O1 tier cheap and predictable.
        if !pass_disabled(&disabled, "simplify") {
            simplify::run_with_config(module, true);
        }
        // Inlining at -O1: GCC's -O1 tier inlines always_inline and
        // extern-inline functions plus called-once/small static helpers, and
        // several torture cases depend on the resulting constant propagation
        // (20010119-1: __builtin_constant_p through extern inline; 990208-1:
        // label-address static inline must duplicate per call site; bcp-1:
        // opt0/opt1 inlined-constant answers). The inliner's own size and
        // budget heuristics bound the expansion. Range-check folding runs
        // after the post-inline promotion below: the fold matches on SSA
        // compare operands, which only exist for actuals once param allocas
        // are promoted (same rationale as the -O2 loop, which runs
        // range_fold after mem2reg).
        if !pass_disabled(&disabled, "inline") {
            inline::run(module);
        }
        // Post-inline parameter promotion: with the inliner done, param
        // allocas can become SSA values. Constants passed through inlined
        // calls (20010119-1: a = 10) become visible to the propagation
        // chain below, exactly like the -O2 tier's post-inline mem2reg.
        crate::ir::mem2reg::promote_allocas_with_params(module);
        // Range-check folding at -O1 (same transform as the -O2 loop's
        // "range_fold" pass): (x >= lo && x <= hi) ->
        // (unsigned)(x - lo) <= (hi - lo) plus the complement form.
        // gcc.c-torture 20041114-1 needs it at -O1; the pass is
        // O(instructions), idempotent, and runs before DCE so folded
        // comparisons feed the same cleanup chain as at -O2.
        if !pass_disabled(&disabled, "range_fold") {
            module.for_each_function(range_check::run_function);
        }
        // Second copy-prop + constant-fold round: simplify rewrites shift
        // identities into Copies (x>>0 => Copy(x)), so comparisons against the
        // original value only become identical-operand compares after the new
        // Copies are propagated (gcc.c-torture shiftopt-1: `(x >> 0) != x`).
        // The -O2+ tier reaches the same fixpoint via its iterative loop; at
        // -O1 this explicit second round keeps the tier loop-free while
        // preserving parity. Both passes are linear and idempotent.
        copy_prop::run(module);
        constant_fold::run(module);
        // Same f128-builtin fold as -O0 (see above).
        simplify::fold_math_intrinsic_calls(module);
        module.for_each_function(dce::eliminate_dead_code);
        // Same-block dead store elimination is cheap and removes the
        // store-overwritten lowering residue at -O1 too.
        module.for_each_function(dse::eliminate_dead_stores);
        module.for_each_function(store_load_forward::run);
        constant_fold::run(module);
        module.for_each_function(cfg_simplify::simplify_cfg);
        // Resolve remaining __builtin_constant_p queries BEFORE the final DCE:
        // an unresolved IsConstant node keeps both select arms alive, so the
        // not-taken arm's calls (20010119-1: undef()) only become removable
        // after resolution. The final DCE then collects them; previously the
        // resolver ran last and the dead branches survived to codegen.
        constant_fold::resolve_remaining_is_constant(module);
        // Post-resolution prune: resolving IsConstant turns previously
        // both-arms-alive diamonds into constant branches, so one more
        // cfg_simplify + DCE round folds the taken edge and deletes the
        // dead arm's blocks (20010119-1: undef() inside the extern-inline
        // body's not-constant arm reached codegen before this round).
        module.for_each_function(cfg_simplify::simplify_cfg);
        module.for_each_function(dce::eliminate_dead_code);
        // Bool-phi merge-diamond branch threading (same call as the -O2+
        // tier's late phase; see there for the rationale).
        if !pass_disabled(&disabled, "boolthread") && std::env::var("CCC_NO_BOOL_THREAD").is_err() {
            let n = module.for_each_function(bool_thread::run);
            if n > 0 {
                module.for_each_function(cfg_simplify::simplify_cfg);
                module.for_each_function(dce::eliminate_dead_code);
            }
        }
        resolve_asm::resolve_inline_asm_symbols(module);
        if std::env::var("CCC_DUMP_IR_AFTER").is_ok() {
            eprintln!("==== IR after all passes (opt_level={}) ====", opt_level);
            eprintln!("{:#?}", module);
            eprintln!("==== END IR after all passes ====");
        }
        return;
    }

    let optimize_for_size = opt_level >= 4;
    // -Os/-Oz are size profiles. Full inlining inflates spill-heavy TUs; but
    // disabling inlining ENTIRELY makes -Os binaries LARGER than -O3 (every
    // memread/memcmp helper emitted standalone). GCC inlines tiny/small callees
    // even at -Os: run the size-aware inliner so helper bodies fold away while
    // repeated cold medium-body expansion stays out. -O1/-O2/-O3 keep full
    // inlining.

    macro_rules! preloop_dump {
        ($name:expr) => {
            if dump_each_pass {
                dump_ir_filtered(module, &format!("pre-loop {}", $name));
            }
            // Structural verification runs independently of dumping; see
            // `passes::verify` (CCC_VERIFY_IR=1 / =abort).
            verify::verify_after_pass(module, $name);
        };
    }
    let allow_inline = opt_level != 4 && opt_level != 5;
    preloop_dump!("lowering(pre-O2)");
    run_inline_phase(module, &disabled, allow_inline, optimize_for_size);
    preloop_dump!("inline_phase");
    if !pass_disabled(&disabled, "fortifyfold") {
        fortify_fold::run(module, target.is_32bit());
        preloop_dump!("fortify_fold");
    }
    // Fold strlen("literal") after inlining so __builtin_constant_p patterns
    // (glibc _startup_fatal) resolve to 1 and the not-constant fallback
    // disappears.
    constant_fold::fold_strlen_literals(module);
    constant_fold::resolve_remaining_is_constant(module);

    // Switch case outlining: extract case bodies from large switch statements
    // (like SQLite's VdbeExec with 170+ cases) into separate functions.
    // OPT-IN ONLY (CCC_OUTLINE_SWITCH=1): the pass miscompiles dispatch
    // loops whose cursor lives in a caller-saved register (see the header
    // comment in outline_switch.rs). Previously the pass ran on every
    // compile with an unreachable threshold (999999), paying a full module
    // walk for guaranteed no-ops; now the walk is skipped entirely unless
    // explicitly requested for experiments.
    if std::env::var("CCC_OUTLINE_SWITCH").is_ok() && !pass_disabled(&disabled, "outline") {
        outline_switch::run(module);
    }
    preloop_dump!("outline");

    // Tail-call-to-loop transformation.
    // Converts self-recursive tail calls into back-edge branches before the
    // main optimization loop so that LICM, IVSR, and GVN can optimize the
    // resulting loops (e.g., hoist invariants, reduce induction variables).
    if !pass_disabled(&disabled, "tce") {
        module.for_each_function(tail_call_elim::tail_calls_to_loops);
    }
    // Collapse the canonical accumulator-sum loop after TCE. This is a
    // semantics-preserving closed form for the strict int/long recurrence and
    // removes O(n) work without touching arbitrary loops.
    if !pass_disabled(&disabled, "tail_sum_formula") {
        module.for_each_function(tail_call_elim::closed_form_tail_sum);
    }
    preloop_dump!("tce");

    // Binary recursion → iterative accumulator (e.g., Fibonacci).
    // Runs after TCE so it catches patterns TCE can't handle (non-tail binary recursion).
    if !pass_disabled(&disabled, "rec2iter") {
        module.for_each_function(recursion_to_iter::recursion_to_iteration);
    }
    preloop_dump!("rec2iter");

    // Post-structural inlining: TCE and recursion-to-iteration can turn a
    // previously recursive static helper into a small, ordinary loop. A second
    // bounded inlining pass exposes constant call arguments and lets the scalar
    // cleanup pipeline fold the caller (e.g. tce_sum's sum(10_000_000, 0)).
    // This is intentionally after the recursion transforms: doing it before
    // them would either reject the recursive callee or clone the call tree.
    //
    // Preserve the size profile here.  The primary inline phase deliberately
    // uses the size-aware policy at -Os/-Oz, so running the unrestricted
    // inliner unconditionally in this later phase defeats that decision and
    // can clone medium helpers at every call site after the first cleanup.
    if !pass_disabled(&disabled, "postinline") {
        if optimize_for_size {
            inline::run_size_optimized(module);
        } else {
            inline::run(module);
        }
        crate::ir::mem2reg::promote_allocas_with_params(module);
        constant_fold::run(module);
        copy_prop::run(module);
        simplify::run(module);
        module.for_each_function(cfg_simplify::run_function);
        module.for_each_function(dce::eliminate_dead_code);
        copy_prop::forward_memcpy_chains(module);
        copy_prop::forward_large_memcpy_loads(module);
    }
    // Aggregate SROA: replace struct/array Memcpy copies with scalar field
    // accesses so the surrounding mem2reg can promote the fields to SSA
    // (by-value struct args/returns and struct assignment after the final
    // inlining). Covers the cases the narrower memcpy forwards miss: copies
    // below the 128-byte large-copy threshold, sources that are GEPs into a
    // local alloca, and the copy-out write path.
    //
    // Enabled by default. It was previously opt-in because the IR it produced
    // violated SSA def-before-use -- the planner's insert and remove indices
    // both referenced the ORIGINAL instruction list, but removals were applied
    // first, so every later insert landed one slot too late and a fresh
    // GetElementPtr ended up AFTER the Load consuming it. The backend then
    // faithfully emitted `movsd (%r11),%xmm5` before `leaq 8(%rdx),%r11` and
    // struct_copy segfaulted. That was misattributed to a backend scheduling
    // defect; the backend is innocent. Edits are now applied in a single
    // index-stable rebuild per block. Two further soundness holes are fixed:
    // the escape scan is an allowlist (Intrinsic operands used to be invisible,
    // which deleted live SIMD copies), and dead aggregate allocas are dropped
    // only after re-checking the final instruction stream.
    if std::env::var("CCC_DISABLE_AGGREGATE_SROA").is_err() {
        aggregate_sroa::run(module);
        crate::ir::mem2reg::promote_allocas_with_params(module);
        constant_fold::run(module);
        copy_prop::run(module);
        module.for_each_function(dce::eliminate_dead_code);
    }
    preloop_dump!("post-structural-inline");

    // Second GlobalAddr CSE: post-structural inlining can clone callers and
    // re-duplicate address materializations after the pre-GVN run.
    // Pass name for CCC_DISABLE_PASSES: "gaddrcse".
    if !pass_disabled(&disabled, "gaddrcse") {
        global_addr_cse::run_module(module);
        module.for_each_function(dce::eliminate_dead_code);
    }

    let iterations = 3;
    let num_funcs = module.functions.len();
    let mut dirty = vec![true; num_funcs];
    let dis = DisabledPasses::from_env(&disabled);

    // `changed` accumulates which functions were modified during each iteration.
    let mut changed = vec![false; num_funcs];

    // Per-pass change counts from the previous iteration, used for skip decisions.
    // Pass indices: 0=cfg1, 1=copyprop1, 2=narrow, 3=simplify, 4=constfold,
    //               5=gvn, 6=licm, 7=ifconv, 8=copyprop2, 9=dce, 10=cfg2
    const NUM_PASSES: usize = 11;
    let mut prev_pass_changes = [usize::MAX; NUM_PASSES]; // MAX = "assume changed" for iter 0

    // Track first iteration's total changes for diminishing-returns early exit.
    let mut iter0_total_changes = 0usize;

    for iter in 0..iterations {
        let mut total_changes = 0usize;
        let mut total_changes_excl_dce = 0usize; // Exclude DCE for diminishing-returns check
        let mut cur_pass_changes = [0usize; NUM_PASSES];

        // Clear the changed accumulator for this iteration
        changed.iter_mut().for_each(|c| *c = false);

        macro_rules! timed_pass {
            ($name:expr, $body:expr) => {{
                if time_passes {
                    let t0 = std::time::Instant::now();
                    let n = $body;
                    let elapsed = t0.elapsed().as_secs_f64();
                    eprintln!(
                        "[PASS] iter={} {}: {:.4}s ({} changes)",
                        iter, $name, elapsed, n
                    );
                    if dump_each_pass {
                        dump_ir_filtered(module, &format!("after iter={} {}", iter, $name));
                    }
                    verify::verify_after_pass(module, $name);
                    n
                } else {
                    let n = $body;
                    if dump_each_pass {
                        dump_ir_filtered(module, &format!("after iter={} {}", iter, $name));
                    }
                    verify::verify_after_pass(module, $name);
                    n
                }
            }};
        }

        // Helper: check if a pass should run based on upstream pass changes.
        // A pass runs if it or any of its upstream passes made changes last iteration.
        // On iteration 0, all passes run (prev_pass_changes are MAX).
        //
        // Pass dependency graph (which passes create opportunities for which):
        //   cfg_simplify → copy_prop, gvn, dce (simpler CFG)
        //   copy_prop → simplify, constfold, gvn, narrow (propagated values)
        //   narrow → simplify, constfold (smaller types)
        //   simplify → constfold, copy_prop, gvn (reduced expressions, folded casts to copies)
        //   constfold → cfg_simplify, copy_prop, dce (constant branches/dead code, folded exprs to copies)
        //   gvn → copy_prop, dce (eliminated redundant computations)
        //   licm → copy_prop, dce (hoisted code)
        //   if_convert → copy_prop, dce (eliminated branches)
        //   dce → cfg_simplify (empty blocks)
        macro_rules! should_run {
            ($self_idx:expr, $($upstream:expr),*) => {{
                prev_pass_changes[$self_idx] > 0 $(|| prev_pass_changes[$upstream] > 0)*
            }};
        }

        // Phase 1: CFG simplification
        // Upstream: constfold (constant branches), dce (empty blocks)
        if !dis.cfg && should_run!(0, 4, 9) {
            let n = timed_pass!(
                "cfg_simplify1",
                run_on_visited(module, &dirty, &mut changed, cfg_simplify::run_function)
            );
            cur_pass_changes[0] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2: Copy propagation
        // Upstream: cfg_simplify (simpler CFG), gvn (eliminated exprs), licm (hoisted code), if_convert
        if !dis.copyprop && should_run!(1, 0, 5, 6, 7) {
            let n = timed_pass!(
                "copy_prop1",
                run_on_visited(module, &dirty, &mut changed, copy_prop::propagate_copies)
            );
            cur_pass_changes[1] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2a: Division-by-constant strength reduction (first iteration only).
        // Replaces slow div/idiv instructions with fast multiply-and-shift sequences.
        // Run early so subsequent passes (narrowing, simplify, constant folding, DCE)
        // can optimize the expanded instruction sequences.
        //
        // Disabled on i686: the pass generates I64 multiply + shift-right-32 sequences
        // to extract the high 32 bits of a widened multiplication. The i686 backend
        // cannot execute 64-bit arithmetic correctly (it truncates to 32 bits), so these
        // sequences produce wrong results. Fall back to hardware idiv/div instead.
        // TODO: Re-enable once i686 has proper 64-bit arithmetic support, or implement
        // a 32-bit-aware variant that uses single-operand imull for mulhi.
        if iter == 0 && !pass_disabled(&disabled, "divconst") && !target.is_32bit() {
            let n = timed_pass!(
                "div_by_const",
                run_on_visited(
                    module,
                    &dirty,
                    &mut changed,
                    div_by_const::div_by_const_function
                )
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2b: Loop unrolling — iter 0 only, before GVN/LICM so that
        // subsequent passes can optimize the unrolled copies.
        // Pass name for CCC_DISABLE_PASSES: "unroll"
        //
        // Loop rotation runs AFTER vectorize+post-unroll+gaddr-cse (see the
        // block below). Running it before vectorize corrupted the
        // vectorizer's base-dependence analysis on the rotated self-loop
        // form (vectorize_iv_dependent_base SIGSEGV, simd_vecreg miscode).
        // Vector bodies contain Vec* intrinsics which `is_cloneable_pure`
        // rejects, so rotation bails on vectorized loops and only fires on
        // the residual scalar counted loops the vectorizer left alone.
        if iter == 0 && opt_level >= 3 && !optimize_for_size && !dis.unroll {
            let n = timed_pass!(
                "loop_unroll",
                run_on_visited(module, &dirty, &mut changed, loop_unroll::unroll_loops)
            );
            total_changes += n;
            total_changes_excl_dce += n;
            if n > 0 && std::env::var("CCC_DISABLE_AGGREGATE_SROA").is_err() {
                aggregate_sroa::run(module);
                crate::ir::mem2reg::promote_allocas_with_params(module);
                constant_fold::run(module);
                copy_prop::run(module);
                module.for_each_function(dce::eliminate_dead_code);
            }
        }

        // Phase 2b-vec: x86-64 vectorization — iter 0 only, EARLY in pipeline.
        // Run before GVN/LICM/etc to catch IR in simpler state.
        // The vectorizer currently emits Vec* intrinsics backed by XMM/YMM
        // registers. Other backends deliberately reject those intrinsics, so
        // running this pass for them turns ordinary scalar loops into a compiler
        // panic instead of preserving the valid scalar program.
        // Pass name for CCC_DISABLE_PASSES: "vectorize"
        // Vectorize: x86-64 gets the full SSE/AVX-shaped pass; AArch64 gets
        // the 128-bit (2-wide F64 / 4-wide I32) NEON variant. -Os/-Oz skip
        // vectorization entirely (code size).
        if iter == 0
            && !optimize_for_size
            && matches!(
                target,
                crate::backend::Target::X86_64 | crate::backend::Target::Aarch64
            )
            && !pass_disabled(&disabled, "vectorize")
        {
            let vectorize_fn = match (target, fp_reassoc, fp_contract, x86_avx) {
                (crate::backend::Target::Aarch64, true, _, _) => {
                    vectorize::vectorize_function_two_wide_fast_math
                }
                (crate::backend::Target::Aarch64, false, _, _) => {
                    vectorize::vectorize_function_two_wide
                }
                (_, true, FpContract::Fast, true) => vectorize::vectorize_function_fast_math,
                (_, true, FpContract::Fast, false) => {
                    vectorize::vectorize_function_fast_math_without_fixed_slp
                }
                (_, true, FpContract::Off | FpContract::OnExpr, true) => {
                    vectorize::vectorize_function_reassoc
                }
                (_, true, FpContract::Off | FpContract::OnExpr, false) => {
                    vectorize::vectorize_function_reassoc_without_fixed_slp
                }
                // contract=fast without reassoc: reductions stay order-locked,
                // map trees may fuse (GCC-parity for -ffp-contract=fast).
                (_, false, FpContract::Fast, _) => vectorize::vectorize_function_contract,
                (_, false, _, _) => vectorize::vectorize_function,
            };
            let n = timed_pass!(
                "vectorize",
                run_on_visited(module, &dirty, &mut changed, vectorize_fn)
            );
            total_changes += n;
            total_changes_excl_dce += n;

            // Single-use vector-load sinking (x86 only): place each stream
            // load directly before its unique consumer so the emitter's
            // VLFOLD window folds it as a memory operand instead of staging
            // it through %ymm0 and a 32-byte spill slot (saxpy-shaped
            // `y[i] = y[i] OP f(x[i])` bodies at -ffp-contract=off).  Runs
            // before the post-vectorize unroll so every unrolled copy
            // inherits the folded order.  Pure reordering of reads; a
            // "changed" signal is not needed downstream.
            // Pass name for CCC_DISABLE_PASSES: "vec_load_sink"
            if target == crate::backend::Target::X86_64
                && !pass_disabled(&disabled, "vec_load_sink")
            {
                let mut scratch = vec![false; module.functions.len()];
                timed_pass!(
                    "vec_load_sink",
                    run_on_visited(module, &dirty, &mut scratch, vec_load_sink::sink_vector_loads)
                );
            }

            // Post-vectorize unroll: the matmul/map vector body is often a
            // single intrinsic — unrolling it 2–4× exposes independent FMA
            // chains (GCC's multi-accum style) without growing scalar code.
            if !dis.unroll {
                let n = timed_pass!(
                    "loop_unroll_post_vec",
                    run_on_visited(module, &dirty, &mut changed, loop_unroll::unroll_loops)
                );
                total_changes += n;
                total_changes_excl_dce += n;
            }

            // Post-vectorize DCE. The vectorizer can bail out of a loop
            // AFTER hoisting setup intrinsics into the preheader (a map
            // loop over a global array left two dead VecBroadcastF64x4
            // defs); no DCE ran again before codegen, and the x86
            // intrinsic emitter ICEs storing a vector value that never
            // received a register or slot home ("value 36 has no
            // register, stack slot, Copy, or GlobalAddr definition" on
            // `for (i...) gd[i] = gd[i]*k + c;` with a global `gd`).
            // Pure intrinsics with an unused dest are DCE-eligible, so
            // one sweep here removes every orphaned setup def regardless
            // of which bailout path produced it.
            module.for_each_function(dce::eliminate_dead_code);
        }

        // Unroll/vectorize can clone loop bodies that still contained
        // GlobalAddr. Re-CSE is O(n) and idempotent once addresses live in entry.
        if iter == 0 && !pass_disabled(&disabled, "gaddrcse") {
            let n = timed_pass!(
                "global_addr_cse_post_unroll",
                global_addr_cse::run_module(module)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2b-ivw: Induction-variable widening — iter 0, BEFORE
        // loop_rotate (so loops are still in guard-at-top form: preheader
        // + header + latch, the shape the pass's phi candidacy expects).
        // Also AFTER vectorize+unroll (vectorizer-managed loops keep
        // their own I32 IVs and are not touched here; the residual scalar
        // counted loops the vectorizer left alone are the widening
        // candidates). Widens I32/U32 IVs whose only 64-bit use is
        // addressing (a Cast I32→I64 that feeds a GEP offset) to I64,
        // eliminating the per-iteration `movslq` the x86 backend must
        // emit after every 32-bit `addl` (the upper 32 bits are clobbered
        // and the next GEP use would read garbage without re-extension).
        // Gated by `CCC_NO_IV_WIDEN`. -O2+, not -Os/-Oz.
        if iter == 0 && opt_level >= 2 && !optimize_for_size {
            let n = timed_pass!(
                "iv_widen",
                run_on_visited(module, &dirty, &mut changed, iv_widen::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Loop-carried store-to-load forwarding — iter 0, AFTER iv_widen
        // (the `int i` address chains are now pointer-width affine forms)
        // and BEFORE loop rotation / IVSR (guard-at-top preheader+header+
        // latch shape, plain `GEP(base, i<<k)` addresses). Turns
        // `a[i] = f(a[i-1])` into a register recurrence: removes the load
        // and the L1D store-forwarding round trip from the loop's critical
        // path (tls_seg_access 2.19× → parity with GCC's tree-predcom).
        // Kill switch `CCC_NO_LOOP_CARRIED_FWD`. -O2+.
        if iter == 0 && opt_level >= 2 && !pass_disabled(&disabled, "loop_carried_forward") {
            let n = timed_pass!(
                "loop_carried_forward",
                loop_carried_forward::run_module(module)
            );
            if n > 0 {
                for c in changed.iter_mut() {
                    *c = true;
                }
                verify::verify_after_pass(module, "loop_carried_forward");
            }
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2b-rot: Loop rotation — iter 0, AFTER iv_widen (so the
        // widened phi flows through the rotated form and emits `addq`
        // directly). Transforms "guard-at-top" loops into "test-at-bottom"
        // self-loop form (one fewer branch per iteration in the hot path;
        // the exit falls through). Every scalar counted loop the vectorizer
        // left alone benefits. The transform is conservative (pure-SSA cond
        // setup only, `is_cloneable_pure` rejects Vec* intrinsics so
        // vectorized loops bail) and the exit-merge-phi reads the body's
        // post-iteration value (latch incoming) on the test-exit edge.
        // -O2+, not -Os/-Oz (cloned cond grows code size). Opt-in via
        // `CCC_LOOP_ROTATE=1`; kill-switch `CCC_NO_LOOP_ROTATE` wins.
        // Runs at iter 0 so the rotated form flows through
        // GVN/LICM/if-convert/dce/cfg_simplify.
        if iter == 0 && opt_level >= 2 && !optimize_for_size {
            let n = timed_pass!(
                "loop_rotate",
                run_on_visited(module, &dirty, &mut changed, loop_rotate::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2c: Integer narrowing
        // Upstream: copy_prop (propagated values expose narrowing)
        if !dis.narrow && should_run!(2, 1) {
            let n = timed_pass!(
                "narrow",
                run_on_visited(module, &dirty, &mut changed, narrow::narrow_function)
            );
            cur_pass_changes[2] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 3: Algebraic simplification
        // Upstream: copy_prop (propagated values), narrow (smaller types)
        if !dis.simplify && should_run!(3, 1, 2) {
            let n = timed_pass!(
                "simplify",
                run_on_visited(module, &dirty, &mut changed, simplify::simplify_function)
            );
            cur_pass_changes[3] = n;
            total_changes += n;
            total_changes_excl_dce += n;

            let enable_bit_reverse = target == crate::backend::Target::Aarch64;
            let n = timed_pass!(
                "bit_idioms",
                run_on_visited(module, &dirty, &mut changed, |func| {
                    bit_idioms::recognize_function(func, enable_bit_reverse)
                })
            );
            cur_pass_changes[3] += n;
            total_changes += n;
            total_changes_excl_dce += n;

            // Correlated value propagation: fold compares, selects and
            // branches whose outcome is implied by dominating conditional
            // edges (GCC evrp / LLVM CVP).  Runs after bit_idioms so the
            // zero guards it materializes are folded on paths that already
            // established non-zero.  Pass name for CCC_DISABLE_PASSES: "cvp".
            if !pass_disabled(&disabled, "cvp") {
                let n = timed_pass!(
                    "cvp",
                    run_on_visited(module, &dirty, &mut changed, cvp::run_function)
                );
                cur_pass_changes[3] += n;
                total_changes += n;
                total_changes_excl_dce += n;
            }
        }

        // Phase 3b: Loop-carried accumulator reassociation.
        // `sum1 += b[i]; sum2 += sum1;` (unsigned) is reassociated into the
        // closed form sum2' = sum2 + N*sum1 + Σ (N-i)*b[i], breaking the serial
        // dependency between the two chains (ICC's Adler-32 rotation). Idempotent
        // (the pattern no longer matches after the rewrite). Pass name for
        // CCC_DISABLE_PASSES: "reassoc_accum".
        if !pass_disabled(&disabled, "reassoc_accum") {
            let n = timed_pass!(
                "reassoc_accum",
                run_on_visited(module, &dirty, &mut changed, reassoc_accum::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 4: Sparse conditional constant propagation, then constant folding
        //
        // SCCP runs first because it is the strictly stronger analysis: it
        // discovers constants that only hold on the *reachable* paths, which
        // `constant_fold` (a local, unconditional folder) cannot see. Feeding
        // it first means the folder and the passes after it work on IR where
        // dead arms are already gone and joins have collapsed.
        //
        // It emits `Copy { dest, Const }` at each proven definition, which is
        // exactly the form `constant_fold` and `copy_prop` already consume, so
        // the two compose without any new plumbing.
        //
        // Gated at -O2+ (including -Os/-Oz, where it is a size win: folded
        // branches delete whole blocks). Kill switch: CCC_DISABLE_PASSES=sccp.
        //
        // Upstream: same producers as constfold — inlining and copy_prop turn
        //           arguments into constants, if_convert creates Selects, and
        //           cfg_simplify exposes newly single-predecessor joins.
        if opt_level >= 2 && !pass_disabled(&disabled, "sccp") && should_run!(4, 0, 1, 2, 3, 7, 8) {
            let n = timed_pass!(
                "sccp",
                run_on_visited(module, &dirty, &mut changed, sccp::run_function)
            );
            cur_pass_changes[4] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Upstream: copy_prop (propagated constants), narrow, simplify (reduced exprs),
        //           if_convert (creates Select that constfold can fold with known-constant cond),
        //           copy_prop2 (propagates constants into Select/Cmp operands after if_convert)
        if !dis.constfold && should_run!(4, 1, 2, 3, 7, 8) {
            let n = timed_pass!(
                "constfold",
                run_on_visited(module, &dirty, &mut changed, constant_fold::fold_function)
            );
            cur_pass_changes[4] += n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phases 5-6a: GVN + LICM + IVSR with shared CFG analysis.
        //
        // These three passes all need CFG + dominator + loop analysis. Since GVN
        // does not modify the CFG (it only replaces instruction operands within
        // existing blocks), the analysis computed for GVN remains valid for LICM
        // and IVSR. We compute it once per function and share it across all three.
        {
            // GVN is enabled; CCC_DISABLE_PASSES=gvn is the diagnostic kill switch.
            let gvn_enabled = true;
            let run_gvn = gvn_enabled && !dis.gvn && should_run!(5, 0, 1, 3);
            let run_licm = !dis.licm && should_run!(6, 0, 1, 5);
            let run_ivsr = iter == 0
                && std::env::var("CCC_NO_IVSR").is_err()
                && !pass_disabled(&disabled, "ivsr");
            // Un-IVSR only pays off on targets with scaled-index addressing
            // (x86-64 SIB). Gated for diagnostics like the other loop passes.
            let run_univsr = run_ivsr
                && matches!(target, crate::backend::Target::X86_64)
                && std::env::var("CCC_NO_UNIVSR").is_err()
                && !pass_disabled(&disabled, "univsr");

            if run_gvn || run_licm || run_ivsr {
                let (gvn_n, licm_n, ivsr_n) = run_gvn_licm_ivsr_shared(
                    module,
                    &dirty,
                    &mut changed,
                    run_gvn,
                    run_licm,
                    run_ivsr,
                    run_univsr,
                    time_passes,
                    iter,
                );
                cur_pass_changes[5] = gvn_n;
                total_changes += gvn_n;
                total_changes_excl_dce += gvn_n;
                cur_pass_changes[6] = licm_n;
                total_changes += licm_n;
                total_changes_excl_dce += licm_n;
                total_changes += ivsr_n;
                total_changes_excl_dce += ivsr_n;
            }
        }

        // Phase 7: If-conversion
        // Upstream: cfg_simplify (simpler CFG), constfold (simplified conditions)
        // -Os/-Oz: skip if-conversion. `cmov` is 4-6 bytes vs a 2-byte jcc +
        // the skipped block, so branches are usually smaller (and often faster
        // on the short branch path); GCC makes the same trade-off at -Os.
        // if_convert also RUNS at -Os: measured on the kernel's real-mode
        // number()/printf.c reproducer, -Os WITHOUT if-conversion is LARGER
        // than -O2 WITH it (1156 vs 955 bytes i686 .text) -- the branchy
        // select lowering produces two arms full of slot round-trips, which
        // is strictly bigger than the setcc/cmov form. GCC if-converts at
        // -Os for the same reason.
        if !dis.ifconv && should_run!(7, 0, 4) {
            // Forward redundant reloads first: removing a load from a
            // conditional arm makes the arm side-effect-free so
            // if-conversion can fire.
            module.for_each_function(load_forward::run);
            verify::verify_after_pass(module, "load_forward");
            let n = timed_pass!(
                "if_convert",
                run_on_visited(
                    module,
                    &dirty,
                    &mut changed,
                    if_convert::if_convert_function
                )
            );
            cur_pass_changes[7] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Re-run bit-idiom recognition after if-conversion. Portable trees such
        // as Linux __ffs are initially CFG diamonds; only this placement sees
        // the parallel value/count Select chains needed for Ctz recognition.
        // The pass is idempotent and remains behind the normal disable switch.
        if !pass_disabled(&disabled, "bit_idioms") {
            let enable_bit_reverse = target == crate::backend::Target::Aarch64;
            let n = timed_pass!(
                "bit_idioms_post_ifconv",
                run_on_visited(module, &dirty, &mut changed, |func| {
                    bit_idioms::recognize_function(func, enable_bit_reverse)
                })
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Redundant zero-guard elimination for Clz/Ctz. If-conversion turns a
        // guarded `x ? __builtin_clz(x) : width` ternary (the k09_clz `lz`
        // kernel shape) into a Select whose intrinsic arm ALREADY yields the
        // width constant at zero — the IR defines Clz(0)/Ctz(0) == operand
        // width. Collapsing the Select onto the intrinsic arm stops codegen
        // from re-selecting the same answer and emitting the dead cmov tail
        // after the bsr/bsf zero fix-up. Idempotent; shares the simplify
        // disable switch.
        if !pass_disabled(&disabled, "simplify") {
            let n = timed_pass!(
                "bitop_zero_guard_fold",
                run_on_visited(
                    module,
                    &dirty,
                    &mut changed,
                    simplify::remove_redundant_bitop_guards
                )
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Correlated value propagation after if-conversion: the post-ifconv
        // bit_idioms placement (Ctz from the __ffs Select chain) emits a
        // `v == 0 ? 63 : ctz(v)` guard whose block is dominated by the
        // `while (!v)` exit edge; only a dominance-scoped fold removes it.
        if !pass_disabled(&disabled, "cvp") {
            let n = timed_pass!(
                "cvp_post_ifconv",
                run_on_visited(module, &dirty, &mut changed, cvp::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 7b: Range-check folding.
        // (x >= lo && x <= hi) -> (unsigned)(x - lo) <= (hi - lo), and the
        // complement (x < lo || x > hi) -> (unsigned)(x - lo) > (hi - lo).
        // Runs right after if-conversion, which produces the Select form from
        // short-circuit branches. Idempotent and O(instructions), so it also
        // folds Selects produced earlier (mem2reg in the pre-loop phase).
        // Pass name for CCC_DISABLE_PASSES: "range_fold".
        if !pass_disabled(&disabled, "range_fold") {
            let n = timed_pass!(
                "range_fold",
                run_on_visited(module, &dirty, &mut changed, range_check::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Sparse small-set membership: `x==C1 || x==C2 || ...` CondBranch
        // chains (multi-way OR stays control flow here — if_convert leaves
        // it, range_fold has already folded contiguous runs) collapse into
        // sub+range-guard+BitTest. GCC's Expat/SQLite classify idiom;
        // consumes the BitTest op (IS-06). Runs right after range_fold so
        // folded ranges join the chain as bit runs.
        // Pass name for CCC_DISABLE_PASSES: "set_membership".
        if !pass_disabled(&disabled, "set_membership") {
            let n = timed_pass!(
                "set_membership",
                run_on_visited(module, &dirty, &mut changed, set_membership::run_function)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 8: Copy propagation again
        // Upstream: simplify (folded casts to copies), constfold (folded exprs to copies),
        //           gvn (produced copies), licm (hoisted code), if_convert (select values)
        // Note: simplify and constfold run earlier in this iteration, so we check
        // cur_pass_changes for them (not just prev_pass_changes via should_run!).
        if !dis.copyprop
            && (should_run!(8, 5, 6, 7) || cur_pass_changes[3] > 0 || cur_pass_changes[4] > 0)
        {
            let n = timed_pass!(
                "copy_prop2",
                run_on_visited(module, &dirty, &mut changed, copy_prop::propagate_copies)
            );
            cur_pass_changes[8] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 9: Dead code elimination
        // Upstream: gvn, licm, if_convert, copy_prop2 (produced dead instructions)
        // Note: DCE changes are excluded from the diminishing-returns comparison
        // (total_changes_excl_dce) because DCE is a cleanup pass that removes dead
        // instructions. Its large change count (often 5000+ in iteration 0) inflates
        // iter0_total_changes, and by removing instructions, DCE actually reduces
        // the work subsequent passes can do in later iterations. This combination
        // causes the diminishing-returns heuristic to exit too early, preventing
        // the optimizer from completing multi-iteration constant propagation chains
        // (e.g., kernel's cpucap_is_possible switch folding through inlined
        // system_supports_sme -> alternative_has_cap_unlikely -> cpucap_is_possible).
        if !dis.dce && should_run!(9, 5, 6, 7, 8) {
            let n = timed_pass!(
                "dce",
                run_on_visited(module, &dirty, &mut changed, dce::eliminate_dead_code)
            );
            cur_pass_changes[9] = n;
            total_changes += n;
            // Intentionally NOT added to total_changes_excl_dce
        }

        // Phase 9.5: Dead store elimination (same-block overwritten stores).
        // Runs right after DCE so stores made dead by folding are gone and
        // the values stored by eliminated instructions feed the next DCE
        // round. Like DCE, its change count is excluded from the
        // diminishing-returns comparison (pure cleanup pass).
        if !dis.dse && !dis.dce && should_run!(9, 5, 6, 7, 8) {
            let n = timed_pass!(
                "dse",
                run_on_visited(module, &dirty, &mut changed, dse::eliminate_dead_stores)
            );
            total_changes += n;
            // Re-run DCE when DSE removed stores: the stored values may now
            // be dead. Cheap and bounded (next iteration's DCE is gated by
            // the same should_run! heuristics).
            if n > 0 {
                let m = timed_pass!(
                    "dce-post-dse",
                    run_on_visited(module, &dirty, &mut changed, dce::eliminate_dead_code)
                );
                total_changes += m;
            }
        }

        // Phase 10: CFG simplification again
        // Upstream: constfold (constant branches), dce (dead blocks), if_convert
        if !dis.cfg && should_run!(10, 4, 7, 9) {
            let n = timed_pass!(
                "cfg_simplify2",
                run_on_visited(module, &dirty, &mut changed, cfg_simplify::run_function)
            );
            cur_pass_changes[10] = n;
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 10.5: Interprocedural constant propagation (IPCP).
        // Run on every iteration, not just iter 0, because later iterations may
        // have simplified call arguments to constants (e.g., phi nodes collapsed
        // after CFG simplification resolved dead branches from IS_ENABLED() checks).
        let mut ipcp_changes = 0;
        if !dis.ipcp {
            ipcp_changes = timed_pass!("ipcp", ipcp::run(module));
            let purity_changes = ip_purity::run(module);
            if ipcp_changes > 0 || purity_changes > 0 {
                changed.iter_mut().for_each(|c| *c = true);
            }
            total_changes += ipcp_changes + purity_changes;
            total_changes_excl_dce += ipcp_changes + purity_changes;
        }

        if iter == 0 {
            iter0_total_changes = total_changes_excl_dce;
        }

        // Early exit: if no passes changed anything, additional iterations are useless.
        if total_changes == 0 {
            break;
        }

        // Diminishing returns: if this iteration produced very few changes relative
        // to the first iteration, another iteration is unlikely to be worthwhile.
        // The optimizer converges quickly: typically iter 0 finds ~264K changes,
        // iter 1 finds ~10K, iter 2 finds ~200. Stopping when an iteration yields
        // less than 5% of the first iteration's output saves one full pipeline
        // iteration with negligible impact on optimization quality.
        //
        // We use total_changes_excl_dce for this comparison because DCE is a
        // cleanup pass whose large change count (removing dead instructions)
        // inflates iter0's total and makes subsequent iterations look like
        // diminishing returns even when they're still making meaningful progress.
        // DCE also reduces the number of instructions available for other passes,
        // naturally lowering their change counts in later iterations.
        //
        // Exception: if IPCP made changes this iteration, always run another
        // iteration regardless of diminishing returns. IPCP changes (constant
        // argument propagation, dead call elimination) create opportunities for
        // constant folding, DCE, and CFG simplification that require a full pass
        // to clean up. Without this, dead code referencing undefined symbols
        // (like the kernel's convert_to_fxsr) would survive.
        // We require iter > 1 (at least 2 full iterations) because multi-step
        // constant propagation chains (e.g., kernel's switch folding through
        // inlined cpucap_is_possible -> alternative_has_cap_unlikely) need at
        // least 2 iterations to complete: iter0 for initial folding, iter1 for
        // propagating results through the control flow.
        const DIMINISHING_RETURNS_FACTOR: usize = 20; // 1/20 = 5% threshold
        if iter > 1
            && ipcp_changes == 0
            && iter0_total_changes > 0
            && total_changes_excl_dce * DIMINISHING_RETURNS_FACTOR < iter0_total_changes
        {
            break;
        }

        // Save per-pass change counts for next iteration's skip decisions.
        prev_pass_changes = cur_pass_changes;

        // Prepare dirty set for next iteration: only re-visit functions that changed.
        std::mem::swap(&mut dirty, &mut changed);
    }

    // Late vectorization (levkropp 8ef1978f concept, audited): Select-
    // shaped reductions (max, conditional sums) only exist after the main
    // loop's if_convert phase, so the early vectorizer never sees them.
    // Re-running the pass here catches the converted form; already-
    // vectorized loops contain Vec* intrinsics and fail the scalar-shape
    // analyzers, so this is idempotent. -Os/-Oz skip it like the early pass.
    // AArch64 reruns the two-wide NEON pass; x86-64 reruns the AVX2 pass
    // (conditional I64 sums are the x86 shape: the widening reduction
    // machinery handles the Select-guard form).
    // CCC_DISABLE_PASSES=latevec disables.
    if matches!(target, crate::backend::Target::Aarch64)
        && !optimize_for_size
        && !pass_disabled(&disabled, "latevec")
        && !pass_disabled(&disabled, "vectorize")
    {
        let n = module.for_each_function(vectorize::vectorize_function_two_wide_late);
        if n > 0 {
            // The vectorizer's block surgery does not maintain source_spans;
            // drop any that no longer align with their block's instructions
            // (debug-info line table would otherwise attribute wrong lines).
            for func in &mut module.functions {
                for block in &mut func.blocks {
                    if !block.source_spans.is_empty()
                        && block.source_spans.len() != block.instructions.len()
                    {
                        block.source_spans.clear();
                    }
                }
            }
            module.for_each_function(|f| {
                let removed = dce::eliminate_dead_code(f);
                if std::env::var("CCC_PROBE_DCE").is_ok() {
                    // Count remaining VecBroadcast dests and their uses.
                    let mut bcasts = Vec::new();
                    for b in &f.blocks {
                        for i in &b.instructions {
                            if let crate::ir::reexports::Instruction::Intrinsic {
                                dest: Some(d),
                                op,
                                ..
                            } = i
                            {
                                if format!("{op:?}").starts_with("VecBroadcast") {
                                    bcasts.push(d.0);
                                }
                            }
                        }
                    }
                    for id in &bcasts {
                        let mut uses = 0u32;
                        for b in &f.blocks {
                            for i in &b.instructions {
                                crate::backend::liveness::for_each_operand_in_instruction(
                                    i,
                                    |op| {
                                        if let crate::ir::reexports::Operand::Value(v) = op {
                                            if v.0 == *id {
                                                uses += 1;
                                            }
                                        }
                                    },
                                );
                            }
                        }
                        eprintln!("[PROBE-DCE] fn={} bcast v{} uses={}", f.name, id, uses);
                    }
                    eprintln!("[PROBE-DCE] fn={} removed={}", f.name, removed);
                }
                removed
            });
        }
    }

    // x86-64 twin: rerun the AVX2 vectorizer for Select-shaped reductions
    // (conditional sums) that if_convert materialized. Same idempotence and
    // cleanup contract as the AArch64 branch above.
    if matches!(target, crate::backend::Target::X86_64)
        && !optimize_for_size
        && !pass_disabled(&disabled, "latevec")
        && !pass_disabled(&disabled, "vectorize")
    {
        let n = module.for_each_function(vectorize::vectorize_function_late);
        if n > 0 {
            for func in &mut module.functions {
                for block in &mut func.blocks {
                    if !block.source_spans.is_empty()
                        && block.source_spans.len() != block.instructions.len()
                    {
                        block.source_spans.clear();
                    }
                }
            }
            module.for_each_function(|f| {
                let removed = dce::eliminate_dead_code(f);
                if std::env::var("CCC_PROBE_DCE").is_ok() {
                    // Count remaining VecBroadcast dests and their uses.
                    let mut bcasts = Vec::new();
                    for b in &f.blocks {
                        for i in &b.instructions {
                            if let crate::ir::reexports::Instruction::Intrinsic {
                                dest: Some(d),
                                op,
                                ..
                            } = i
                            {
                                if format!("{op:?}").starts_with("VecBroadcast") {
                                    bcasts.push(d.0);
                                }
                            }
                        }
                    }
                    for id in &bcasts {
                        let mut uses = 0u32;
                        for b in &f.blocks {
                            for i in &b.instructions {
                                crate::backend::liveness::for_each_operand_in_instruction(
                                    i,
                                    |op| {
                                        if let crate::ir::reexports::Operand::Value(v) = op {
                                            if v.0 == *id {
                                                uses += 1;
                                            }
                                        }
                                    },
                                );
                            }
                        }
                        eprintln!("[PROBE-DCE] fn={} bcast v{} uses={}", f.name, id, uses);
                    }
                    eprintln!("[PROBE-DCE] fn={} removed={}", f.name, removed);
                }
                removed
            });
        }
    }

    // DCE can remove aggregate-copy consumers that previously made forwarding
    // unsafe (for example a full returned struct whose only surviving use is
    // one field load). Re-run forwarding before final loop promotion.
    // Honors the same kill switch as the first invocation (a gate that only
    // silences one of two callsites is a debugging trap, not a gate).
    if !std::env::var("CCC_NO_AGG_COPY_FWD").is_ok() {
        for _ in 0..8 {
            if module.for_each_function(aggregate_copy_forward::run) == 0 {
                break;
            }
        }
    }
    // Forward alloca-field stores to later loads of the same constant-offset
    // field (the SROA effect for scalarized aggregates: struct memory traffic
    // becomes SSA dataflow). Hardened rewrite of levkropp's 0980060d pass:
    // default-closed transfer function, volatile/escape/segment guards.
    // Iterate: each round can expose new dead stores for the next.
    if !pass_disabled(&disabled, "slforward") {
        for _ in 0..4 {
            if module.for_each_function(store_load_forward::run) == 0 {
                break;
            }
        }
    }
    module.for_each_function(dce::eliminate_dead_code);
    if std::env::var("LCCC_DUMP_IR_PROMOTE").is_ok() {
        for func in &module.functions {
            if func.is_declaration {
                continue;
            }
            eprintln!("=== IR(pre-promote) {} ===", func.name);
            for (bi, b) in func.blocks.iter().enumerate() {
                eprintln!("  block {} (label {}):", bi, b.label.0);
                for inst in &b.instructions {
                    eprintln!("    {:?}", inst);
                }
                eprintln!("    term: {:?}", b.terminator);
            }
        }
    }
    // Run memory-recurrence promotion again after CFG and copy cleanup have
    // exposed canonical natural loops.
    if !std::env::var("CCC_NO_LOOP_MEM_PROMOTE").is_ok() {
        module.for_each_function(loop_memory_promote::run);
        module.for_each_function(loop_memory_promote::mark_f64_add_reduction);
    }
    // Late loop-carried forwarding: IVSR / CFG cleanup expose marching-pointer
    // recurrences that the early run could not normalise.
    loop_carried_forward::run_module(module);
    verify::verify_after_pass(module, "loop_carried_forward_late");
    // Late redundant-load elimination: post-IVSR field accesses have constant
    // offsets, so same-address loads merge when intervening stores are
    // provably non-aliasing (volatile loads are exempt by construction).
    if !pass_disabled(&disabled, "redundantloads") {
        module.for_each_function(redundant_loads::run);
        // Merged loads orphan their (now dead) address computations.
        module.for_each_function(dce::eliminate_dead_code);
    }
    // Second-order strength reduction for slope-1 triangular loop indices
    // (t(t+1)/2 recurrences): carries the index as two counters instead of
    // recomputing mul+corrected-div every iteration. Exact: t(t+1) is always
    // even, so the accumulator is bit-identical including wraparound.
    if !pass_disabled(&disabled, "quadsr") {
        module.for_each_function(quadratic_sr::run);
        module.for_each_function(dce::eliminate_dead_code);
    }

    // Hoist FP constants used in loop bodies into preheader Copies so they can
    // stay in FP registers across the loop (AArch64 constant-pool literal loads
    // otherwise sit in the loop's dependency path every iteration).
    if !pass_disabled(&disabled, "fpconst") {
        module.for_each_function(fp_const_hoist::run);
    }

    // Hoist large integer constants (not encodable as target immediates) out
    // of loop bodies. Target-aware since the Lev-Kropp-audit session:
    //
    // * AArch64: anything outside imm12/cmn pays movz/movk per iteration
    //   (sieve's loop bound was the original case).
    // * x86-64: imm32 is directly encodable in cmp/add/imul, so the pass now
    //   hoists ONLY true 64-bit constants — dominated by div_by_const's
    //   magic multipliers (`movabsq $2454267027` was re-materialized inside
    //   every division loop iteration; sum_div7 paid it per element).
    //   Small imm32 constants are deliberately NOT hoisted (they are free
    //   in-place; hoisting them burns a register and a callee-saved push —
    //   the old sieve regression the AArch64-only gate existed for).
    // * i686: no-op (nothing this pass sees pays a 64-bit materialization).
    if !pass_disabled(&disabled, "intconst")
        && matches!(
            target,
            crate::backend::Target::Aarch64 | crate::backend::Target::X86_64
        )
    {
        int_const_hoist::set_target_aarch64(target == crate::backend::Target::Aarch64);
        module.for_each_function(int_const_hoist::run);
    }

    if !pass_disabled(&disabled, "bepre") {
        let n = module.for_each_function(backedge_pre::run);
        if n > 0 {
            module.for_each_function(dce::eliminate_dead_code);
        }
    }

    // Phase 10.9: Bool-phi merge-diamond branch threading.
    // Value-context short-circuits (`int ok = a && b; if (ok) ...`) and
    // explicit `if (a) ok = 1; else ok = cmp;` merges keep a phi-only block
    // whose sole terminator is a branch on the phi. With if-conversion off
    // (-m16 size profile) that diamond survives every pipeline phase and
    // phi elimination materializes one arm store per predecessor plus a
    // reload-and-test in the merge (~18 bytes per site, ~40 sites in the
    // boot corpus). GCC's jump threader folds it while still SSA; this is
    // the same transform: each predecessor branches on its own incoming
    // value, the phi, the stores, the merge test, and (all preds threaded)
    // the merge block die. IR-level, so every backend benefits; strictly
    // fewer executed instructions on the threaded paths (no store, no
    // reload, no extra branch) — a size and speed win at every tier.
    // Fail-closed soundness rules in the pass header; nothing is ever
    // duplicated. Kill switches: CCC_NO_BOOL_THREAD=1,
    // CCC_DISABLE_PASSES=boolthread.
    if !pass_disabled(&disabled, "boolthread") && std::env::var("CCC_NO_BOOL_THREAD").is_err() {
        let n = module.for_each_function(bool_thread::run);
        if n > 0 {
            // Threaded merges leave unreachable phi-only blocks and dead
            // arm computations behind; cfg_simplify removes the blocks and
            // DCE collects the dead def chains (compare feeds, arm values).
            module.for_each_function(cfg_simplify::simplify_cfg);
            module.for_each_function(|f| {
                let removed = dce::eliminate_dead_code(f);
                if std::env::var("CCC_PROBE_DCE").is_ok() {
                    // Count remaining VecBroadcast dests and their uses.
                    let mut bcasts = Vec::new();
                    for b in &f.blocks {
                        for i in &b.instructions {
                            if let crate::ir::reexports::Instruction::Intrinsic {
                                dest: Some(d),
                                op,
                                ..
                            } = i
                            {
                                if format!("{op:?}").starts_with("VecBroadcast") {
                                    bcasts.push(d.0);
                                }
                            }
                        }
                    }
                    for id in &bcasts {
                        let mut uses = 0u32;
                        for b in &f.blocks {
                            for i in &b.instructions {
                                crate::backend::liveness::for_each_operand_in_instruction(
                                    i,
                                    |op| {
                                        if let crate::ir::reexports::Operand::Value(v) = op {
                                            if v.0 == *id {
                                                uses += 1;
                                            }
                                        }
                                    },
                                );
                            }
                        }
                        eprintln!("[PROBE-DCE] fn={} bcast v{} uses={}", f.name, id, uses);
                    }
                    eprintln!("[PROBE-DCE] fn={} removed={}", f.name, removed);
                }
                removed
            });
        }
    }

    // Phase 11: Dead static function elimination.
    // After all optimizations, remove internal-linkage (static) functions that are
    // never referenced by any other function or global initializer. This is critical
    // for `static inline` functions from headers: after intra-procedural optimizations
    // eliminate dead code paths (e.g., `if (1 || expr)` removes the else branch),
    // some static inline callees may become completely unreferenced and can be removed.
    // Without this, the dead functions may reference undefined external symbols
    // (e.g., kernel's `___siphash_aligned` calling `__siphash_aligned` which doesn't
    // exist on x86 where CONFIG_HAVE_EFFICIENT_UNALIGNED_ACCESS is set).
    dead_statics::eliminate_dead_static_functions(module);

    // Phase 11b: Vector temp promotion. Runs on the final IR (after inlining and
    // the optimization loop) so every vector intrinsic chain is seen whole. This
    // removes the temp alloca + Memcpy that vector intrinsic lowering introduces
    // for `__m256i x = _mm256_*(...)`, writing results directly into the variable
    // slot and enabling the backend's memory-operand folding.
    if !pass_disabled(&disabled, "vecpromote") {
        vector_temp_promotion::promote_vector_temps(module);
    }

    // Phase 11c: Forward direct vector loads into compatible consumers while
    // the original alignment facts are still available. This must precede
    // alignment relaxation: deleting a load can make its temporary's expensive
    // >16-byte alignment completely unobservable.
    if !pass_disabled(&disabled, "vecpromote") {
        vector_temp_promotion::fuse_vector_loads(module);
    }

    // Phase 11d: Relax only the alignment left unobservable by the final use
    // set. Aligned/non-temporal, atomic, volatile, address-observing and
    // unaudited intrinsic positions retain their required alignment.
    if !pass_disabled(&disabled, "vecpromote") {
        vector_temp_promotion::downgrade_nonescaping_vector_align(module);
    }

    if std::env::var("CCC_DUMP_IR_AFTER").is_ok() {
        eprintln!("==== IR after all passes (opt_level={}) ====", opt_level);
        eprintln!("{:#?}", module);
        eprintln!("==== END IR after all passes ====");
    }
}

#[cfg(test)]
mod m16_size_policy_tests {
    use super::apply_m16_size_policy;

    #[test]
    fn policy_is_limited_to_size_optimized_code16() {
        for (code16, level) in [(false, 4), (true, 0), (true, 2), (false, 5)] {
            let mut disabled = String::new();
            apply_m16_size_policy(&mut disabled, code16, level);
            assert!(disabled.is_empty(), "code16={code16} level={level}");
        }

        let mut disabled = "vectorize".to_string();
        apply_m16_size_policy(&mut disabled, true, 4);
        assert_eq!(disabled, "vectorize,postinline,ifconv,gaddrcse,licm");
    }

    // ── restore_phi_prefix ──────────────────────────────────────────────────

    mod phi_prefix {
        use super::super::restore_phi_prefix;
        use crate::common::types::IrType;
        use crate::ir::reexports::{
            BasicBlock, BlockId, Instruction, IrConst, Operand, Terminator, Value,
        };

        fn phi(dest: u32) -> Instruction {
            Instruction::Phi {
                dest: Value(dest),
                ty: IrType::I32,
                incoming: vec![(Operand::Const(IrConst::I32(0)), BlockId(0))],
            }
        }

        fn copy(dest: u32) -> Instruction {
            Instruction::Copy {
                dest: Value(dest),
                src: Operand::Const(IrConst::I32(1)),
            }
        }

        fn block(instructions: Vec<Instruction>) -> BasicBlock {
            BasicBlock {
                label: BlockId(1),
                instructions,
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            }
        }

        fn shape(b: &BasicBlock) -> Vec<(bool, u32)> {
            b.instructions
                .iter()
                .map(|i| match i {
                    Instruction::Phi { dest, .. } => (true, dest.0),
                    _ => (false, i.dest().map(|d| d.0).unwrap_or(u32::MAX)),
                })
                .collect()
        }

        #[test]
        fn an_already_contiguous_block_is_left_alone() {
            let mut b = block(vec![phi(1), phi(2), copy(3), copy(4)]);
            let before = shape(&b);
            assert!(!restore_phi_prefix(&mut b));
            assert_eq!(shape(&b), before);
        }

        #[test]
        fn a_block_with_no_phis_is_left_alone() {
            let mut b = block(vec![copy(3), copy(4)]);
            assert!(!restore_phi_prefix(&mut b));
            assert_eq!(shape(&b), vec![(false, 3), (false, 4)]);
        }

        #[test]
        fn an_empty_block_is_left_alone() {
            let mut b = block(vec![]);
            assert!(!restore_phi_prefix(&mut b));
        }

        #[test]
        fn a_stranded_phi_is_lifted_above_the_copy() {
            // Exactly what univsr and cfg_simplify produce: one phi of several
            // is rewritten into a Copy in place, stranding those after it.
            let mut b = block(vec![phi(1), copy(9), phi(2), phi(3)]);
            assert!(restore_phi_prefix(&mut b));
            assert_eq!(shape(&b), vec![(true, 1), (true, 2), (true, 3), (false, 9)]);
        }

        #[test]
        fn relative_order_is_preserved_within_each_group() {
            let mut b = block(vec![copy(7), phi(1), copy(8), phi(2), copy(9)]);
            assert!(restore_phi_prefix(&mut b));
            assert_eq!(
                shape(&b),
                vec![(true, 1), (true, 2), (false, 7), (false, 8), (false, 9)]
            );
        }

        #[test]
        fn source_spans_are_permuted_with_their_instructions() {
            use crate::common::source::Span;
            let mut b = block(vec![phi(1), copy(9), phi(2)]);
            // Distinguishable spans, parallel to the instructions.
            let mk = |start: u32| Span::new(start, start + 1, 0);
            b.source_spans = vec![mk(10), mk(20), mk(30)];
            assert!(restore_phi_prefix(&mut b));
            assert_eq!(shape(&b), vec![(true, 1), (true, 2), (false, 9)]);
            // The span must follow its instruction, or -g emits .loc against
            // the wrong line.
            let starts: Vec<u32> = b.source_spans.iter().map(|s| s.start).collect();
            assert_eq!(starts, vec![10, 30, 20]);
        }

        #[test]
        fn a_desynchronised_span_vector_is_left_untouched() {
            // Passes have been observed to drop instructions without shrinking
            // source_spans; permuting a mismatched vector would make it worse.
            let mut b = block(vec![phi(1), copy(9), phi(2)]);
            b.source_spans = Vec::new();
            assert!(restore_phi_prefix(&mut b));
            assert!(b.source_spans.is_empty());
        }
    }
}
