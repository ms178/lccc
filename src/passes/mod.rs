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

pub(crate) mod cfg_simplify;
pub(crate) mod constant_fold;
pub(crate) mod copy_prop;
pub(crate) mod dce;
mod dead_statics;
pub(crate) mod div_by_const;
pub(crate) mod gvn;
pub(crate) mod if_convert;
pub(crate) mod inline;
pub(crate) mod ipcp;
pub(crate) mod iv_strength_reduce;
pub(crate) mod licm;
pub(crate) mod loop_analysis;
pub(crate) mod loop_unroll;
pub(crate) mod narrow;
pub(crate) mod outline_switch;
pub(crate) mod recursion_to_iter;
mod resolve_asm;
pub(crate) mod simplify;
pub(crate) mod tail_call_elim;
pub(crate) mod univsr;
pub(crate) mod vector_temp_promotion;
pub(crate) mod vectorize;

use crate::ir::analysis::CfgAnalysis;
use crate::common::fx_hash::FxHashSet;
use crate::ir::reexports::{Instruction, IrFunction, IrModule};

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
        // pass forced ON. Opt-in (CCC_IVSR=1) because gzip measured slightly
        // negative: -6 +1.25% (faster 2/9), -9 +0.17% (faster 3/9).
        if run_ivsr {
            let n = iv_strength_reduce::ivsr_with_analysis(func, &cfg);
            if n > 0 {
                ivsr_total += n;
                if i < changed.len() {
                    changed[i] = true;
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
    ipcp: bool,
    unroll: bool,
}

impl DisabledPasses {
    fn from_env(disabled: &str) -> Self {
        DisabledPasses {
            cfg: disabled.contains("cfg"),
            copyprop: disabled.contains("copyprop"),
            narrow: disabled.contains("narrow"),
            simplify: disabled.contains("simplify"),
            constfold: disabled.contains("constfold"),
            gvn: disabled.contains("gvn"),
            licm: disabled.contains("licm"),
            ifconv: disabled.contains("ifconv"),
            dce: disabled.contains("dce"),
            ipcp: disabled.contains("ipcp"),
            unroll: disabled.contains("unroll"),
        }
    }
}

/// Run Phase 0: function inlining and post-inline optimization passes.
fn run_inline_phase(module: &mut IrModule, disabled: &str, allow_inline: bool, size_profile: bool) {
    let dump_pre = std::env::var("CCC_DUMP_EACH_PASS").is_ok();
    macro_rules! iphase_dump {
        ($name:expr) => {
            if dump_pre {
                eprintln!("==== IR pre-loop: {} ====", $name);
                eprintln!("{:#?}", module);
                eprintln!("==== END IR pre-loop: {} ====", $name);
            }
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
    if !disabled.contains("gvn") {
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

    if !disabled.contains("inline") {
        if size_profile {
            // -Os/-Oz: tiny/small callees inline (helper bodies fold away);
            // normal/medium callees stay out to keep code size down.
            inline::run_small_only(module);
        } else if allow_inline {
            inline::run(module);
        }
    }
    if std::env::var("CCC_DUMP_IR").is_ok() || std::env::var("CCC_DUMP_EACH_PASS").is_ok() {
        eprintln!("==== IR pre-loop: after inliner ====");
        eprintln!("{:#?}", module);
        eprintln!("==== END IR pre-loop: after inliner ====");
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
    if !std::env::var("CCC_NO_CLEANUP_FOLD1").is_ok() { constant_fold::run(module); }
    iphase_dump!("cleanup-fold1");
    if !std::env::var("CCC_NO_CLEANUP_CP1").is_ok() { copy_prop::run(module); }
    iphase_dump!("cleanup-copyprop1");
    if !std::env::var("CCC_NO_CLEANUP_SIMP").is_ok() { simplify::run(module); }
    iphase_dump!("cleanup-simplify");
    if !std::env::var("CCC_NO_CLEANUP_FOLD2").is_ok() { constant_fold::run(module); }
    if !std::env::var("CCC_NO_CLEANUP_CP2").is_ok() { copy_prop::run(module); }
    iphase_dump!("cleanup-fold2-copyprop2");
    resolve_asm::resolve_inline_asm_symbols(module);
    iphase_dump!("post-inline cleanup");
}

/// Run optimization passes for the requested optimization level.
///
/// `opt_level`: 0=-O0, 1=-O1, 2=-O2, 3=-O3, 4=-Os, 5=-Oz.
pub(crate) fn run_passes(module: &mut IrModule, opt_level: u32, target: crate::backend::Target) {
    let disabled = std::env::var("CCC_DISABLE_PASSES").unwrap_or_default();
    // Debug hook: dump the module IR to stderr before optimization.
    if std::env::var("CCC_DUMP_IR").is_ok() {
        eprintln!("==== IR before passes (opt_level={}) ====", opt_level);
        eprintln!("{:#?}", module);
        eprintln!("==== END IR ====");
    }
    if disabled.contains("all") {
        return;
    }

    let time_passes = std::env::var("CCC_TIME_PASSES").is_ok();
    // -fdump-tree-all equivalent: dump the module after every pass.
    let dump_each_pass = std::env::var("CCC_DUMP_EACH_PASS").is_ok();

    // -O0: preserve the alloca-based IR and skip the optimizer completely.
    // Inline asm symbol resolution is not an optimization; it is required for
    // correct backend emission of asm operands.
    if opt_level == 0 {
        resolve_asm::resolve_inline_asm_symbols(module);
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
        crate::ir::mem2reg::promote_allocas_with_params(module);
        if time_passes {
            eprintln!("[PASS] o1 mem2reg");
        }
        constant_fold::run(module);
        copy_prop::run(module);
        // Same f128-builtin fold as -O0 (see above).
        simplify::fold_math_intrinsic_calls(module);
        module.for_each_function(dce::eliminate_dead_code);
        resolve_asm::resolve_inline_asm_symbols(module);
        constant_fold::resolve_remaining_is_constant(module);
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
    // even at -Os: run the small-only inliner so helper bodies fold away while
    // normal/medium callees stay out. -O1/-O2/-O3 keep full inlining.

macro_rules! preloop_dump {
    ($name:expr) => {
        if dump_each_pass {
            eprintln!("==== IR after pre-loop {} ====", $name);
            eprintln!("{:#?}", module);
            eprintln!("==== END IR after pre-loop {} ====", $name);
        }
    };
}
    let allow_inline = opt_level != 4 && opt_level != 5;
    preloop_dump!("lowering(pre-O2)");
    run_inline_phase(module, &disabled, allow_inline, optimize_for_size);
    preloop_dump!("inline_phase");
    // Fold strlen("literal") after inlining so __builtin_constant_p patterns
    // (glibc _startup_fatal) resolve to 1 and the not-constant fallback
    // disappears.
    constant_fold::fold_strlen_literals(module);
    constant_fold::resolve_remaining_is_constant(module);

    // Switch case outlining: extract case bodies from large switch statements
    // (like SQLite's VdbeExec with 170+ cases) into separate functions.
    // Runs after inlining so we see the final function sizes, and before all
    // other passes so the smaller outlined functions benefit from the full
    // optimization pipeline. Pass name for CCC_DISABLE_PASSES: "outline"
    if !disabled.contains("outline") {
        outline_switch::run(module);
    }
    preloop_dump!("outline");

    // Tail-call-to-loop transformation.
    // Converts self-recursive tail calls into back-edge branches before the
    // main optimization loop so that LICM, IVSR, and GVN can optimize the
    // resulting loops (e.g., hoist invariants, reduce induction variables).
    if !disabled.contains("tce") {
        module.for_each_function(tail_call_elim::tail_calls_to_loops);
    }
    // Collapse the canonical accumulator-sum loop after TCE. This is a
    // semantics-preserving closed form for the strict int/long recurrence and
    // removes O(n) work without touching arbitrary loops.
    if !disabled.contains("tail_sum_formula") {
        module.for_each_function(tail_call_elim::closed_form_tail_sum);
    }
    preloop_dump!("tce");

    // Binary recursion → iterative accumulator (e.g., Fibonacci).
    // Runs after TCE so it catches patterns TCE can't handle (non-tail binary recursion).
    if !disabled.contains("rec2iter") {
        module.for_each_function(recursion_to_iter::recursion_to_iteration);
    }
    preloop_dump!("rec2iter");

    // Post-structural inlining: TCE and recursion-to-iteration can turn a
    // previously recursive static helper into a small, ordinary loop. A second
    // bounded inlining pass exposes constant call arguments and lets the scalar
    // cleanup pipeline fold the caller (e.g. tce_sum's sum(10_000_000, 0)).
    // This is intentionally after the recursion transforms: doing it before
    // them would either reject the recursive callee or clone the call tree.
    if !disabled.contains("postinline") {
        inline::run(module);
        crate::ir::mem2reg::promote_allocas_with_params(module);
        constant_fold::run(module);
        copy_prop::run(module);
        simplify::run(module);
        module.for_each_function(cfg_simplify::run_function);
        module.for_each_function(dce::eliminate_dead_code);
        copy_prop::forward_memcpy_chains(module);
        copy_prop::forward_large_memcpy_loads(module);
    }
    preloop_dump!("post-structural-inline");

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
                        eprintln!("==== IR after iter={} {} ====", iter, $name);
                        eprintln!("{:#?}", module);
                        eprintln!("==== END IR after iter={} {} ====", iter, $name);
                    }
                    n
                } else {
                    let n = $body;
                    if dump_each_pass {
                        eprintln!("==== IR after iter={} {} ====", iter, $name);
                        eprintln!("{:#?}", module);
                        eprintln!("==== END IR after iter={} {} ====", iter, $name);
                    }
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
        if iter == 0 && !disabled.contains("divconst") && !target.is_32bit() {
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
        if iter == 0 && opt_level >= 3 && !optimize_for_size && !dis.unroll {
            let n = timed_pass!(
                "loop_unroll",
                run_on_visited(module, &dirty, &mut changed, loop_unroll::unroll_loops)
            );
            total_changes += n;
            total_changes_excl_dce += n;
        }

        // Phase 2b-vec: SSE2 vectorization — iter 0 only, EARLY in pipeline.
        // Run before GVN/LICM/etc to catch IR in simpler state.
        // Pass name for CCC_DISABLE_PASSES: "vectorize"
        if iter == 0 && !optimize_for_size && !disabled.contains("vectorize") {
            let n = timed_pass!(
                "vectorize",
                run_on_visited(module, &dirty, &mut changed, vectorize::vectorize_function)
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
        }

        // Phase 4: Constant folding
        // Upstream: copy_prop (propagated constants), narrow, simplify (reduced exprs),
        //           if_convert (creates Select that constfold can fold with known-constant cond),
        //           copy_prop2 (propagates constants into Select/Cmp operands after if_convert)
        if !dis.constfold && should_run!(4, 1, 2, 3, 7, 8) {
            let n = timed_pass!(
                "constfold",
                run_on_visited(module, &dirty, &mut changed, constant_fold::fold_function)
            );
            cur_pass_changes[4] = n;
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
                && std::env::var("CCC_IVSR").is_ok()
                && !disabled.contains("ivsr");

            if run_gvn || run_licm || run_ivsr {
                let (gvn_n, licm_n, ivsr_n) = run_gvn_licm_ivsr_shared(
                    module,
                    &dirty,
                    &mut changed,
                    run_gvn,
                    run_licm,
                    run_ivsr,
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
        if !dis.ifconv && !optimize_for_size && should_run!(7, 0, 4) {
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
            if ipcp_changes > 0 {
                changed.iter_mut().for_each(|c| *c = true);
            }
            total_changes += ipcp_changes;
            total_changes_excl_dce += ipcp_changes;
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
    if !disabled.contains("vecpromote") {
        vector_temp_promotion::promote_vector_temps(module);
    }

    // Phase 11b: downgrade the alignment of non-escaping vector-sized allocas
    // (16/32 B) so every access skips the runtime lea/add/and alignment dance
    // (3 instructions per 32-byte access in zlib-ng's compare256_avx2 inner
    // loop). All emitter accesses are unaligned moves, and an address that
    // never escapes makes _Alignas unobservable — so the downgrade is sound.
    if !disabled.contains("vecpromote") {
        vector_temp_promotion::downgrade_nonescaping_vector_align(module);
    }

    // Phase 11c: Vector load fusion. After promotion, `ymm = _mm256_loadu(p)`
    // followed by a consumer that reads `ymm` is a pure slot round-trip; fuse
    // the load into the consumer by passing `p` through (the backend emits
    // `vmovdqu (%p)` / uses `(%p)` as the memory operand, matching GCC).
    if !disabled.contains("vecpromote") {
        vector_temp_promotion::fuse_vector_loads(module);
    }

    if std::env::var("CCC_DUMP_IR_AFTER").is_ok() {
        eprintln!("==== IR after all passes (opt_level={}) ====", opt_level);
        eprintln!("{:#?}", module);
        eprintln!("==== END IR after all passes ====");
    }
}
