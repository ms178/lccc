//! Function inlining pass.
//!
//! Inlines small static/static-inline functions and `__attribute__((always_inline))`
//! functions at their call sites. Normal inlining is critical for eliminating dead
//! branches guarded by constant-returning inline functions (e.g., kernel's
//! `IS_ENABLED()` patterns). Always-inline is critical for kernel code where
//! functions must remain in their caller's section (e.g., `.noinstr.text`).
//!
//! After inlining, subsequent passes (constant fold, DCE, CFG simplify) clean up
//! the inlined code and eliminate dead branches.

use crate::common::asm_constraints::constraint_is_immediate_only;
use crate::common::fx_hash::FxHashMap as FxInlineMap;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, EightbyteClass, IrType, RiscvFloatClass};
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, GlobalInit, Instruction, IrBinOp, IrConst, IrFunction, IrModule,
    Operand, Terminator, Value,
};
use crate::passes::loop_analysis;

/// Maximum number of IR instructions (across all blocks) in a callee for it
/// to be eligible for inlining. This handles constant-returning helpers
/// like IS_ENABLED() wrappers and small accessor functions, as well as
/// moderately-sized static inline functions with simple control flow.
/// Lowered from 60 to 32: for user-space workloads like gzip, inlining
/// up-to-60-instruction callees adds large amounts of code (gzip -O3 text was
/// 437KB vs 343KB with inlining off) with no measurable runtime benefit,
/// because the from-scratch backend spills heavily in large functions.
/// Best measured gzip Pareto: 395KB text, 1378ms compress (faster than both the
/// original 60/800 and a more-aggressive 20/250 config).
const MAX_INLINE_INSTRUCTIONS: usize = 32;

/// Separate cap for medium static helpers; caller budgets bound growth.
const MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS: usize = 160;
/// Medium static helpers may contain a small loop nest. Keep this separate from
/// the ordinary six-block limit: aggregate-producing helpers such as
/// `make_group` need their loop CFG inlined before scalar cleanup can expose
/// their hot arithmetic. The caller budget and caller-size caps remain the
/// primary growth controls.
const MAX_MEDIUM_STATIC_INLINE_BLOCKS: usize = 16;

/// A static helper containing a loop can still be a profitable inline candidate
/// when it is small enough to fit the caller cap.  Keeping this separate from
/// the generic 6-block loop cap avoids admitting arbitrary externally-visible
/// loop bodies, while allowing internal checksum/parser kernels to expose
/// constant arguments and post-inline simplification.
///
/// There are two tiers. Tiny loop helpers are cheap enough to clone freely.
/// Larger loop helpers may be cloned at no more than two direct call sites;
/// this retains constant-argument specialization for checksum kernels while
/// preventing medium-sized search loops from duplicating into every caller.
const MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS: usize = 40;
const MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS: usize = 8;
// The fifth clone of a non-explicit static loop helper was materially worse
// than an outlined call in the measured glibc memcmp shape. Keep a narrow
// hard cap until profile data can replace this conservative pressure proxy.
const MAX_SMALL_STATIC_LOOP_INLINE_CLONES: usize = 4;
const MAX_STATIC_LOOP_INLINE_INSTRUCTIONS: usize = 128;
const MAX_STATIC_LOOP_INLINE_BLOCKS: usize = 16;
const MAX_STATIC_LOOP_INLINE_CALLS: usize = 2;
/// At -Os, larger loop bodies stay out of enclosing loops even when inlining
/// their final call site would remove the standalone copy. LCCC's current
/// register allocator spills the merged nest heavily (zlib-ng Adler-32).
const MAX_SIZE_NESTED_LOOP_INLINE_INSTRUCTIONS: usize = 64;

/// Cap for `static inline` functions whose bodies are dominated by SIMD vector
/// intrinsics. Such functions (e.g. zlib-ng's compare256_avx2_static, ~70 IR
/// instructions, 10 blocks) are pathological when NOT inlined: the from-scratch
/// vector codegen materialises every operand through memory, and a per-candidate
/// call in a match loop (longest_match_avx2) is several times slower than the
/// inlined body. GCC always inlines `static inline` regardless of size; we bound
/// the growth with the per-caller budget.
const MAX_VECTOR_STATIC_INLINE_INSTRUCTIONS: usize = 200;
const MAX_VECTOR_STATIC_INLINE_BLOCKS: usize = 24;

/// Maximum number of basic blocks in a callee for inlining eligibility.
/// Must be high enough to handle static inline functions with control flow
/// (e.g., if/else chains, early returns). GCC inlines these aggressively.
const MAX_INLINE_BLOCKS: usize = 6;

/// Higher block limit for callees that contain no back-edges (no loops).
/// Functions with if/else chains, switch statements, or early returns can
/// have many blocks without loops. These are safe to inline more aggressively
/// since they don't create nested loop structures that overwhelm codegen.
///
/// 16 is an experimental threshold: Expat's tiny `xml_name_start` predicate
/// lowers to 13 acyclic blocks despite being a single source expression.  The
/// benchmark/hotspot suite validates whether admitting this class improves
/// generated code without unacceptable broad code-growth.
const MAX_INLINE_BLOCKS_NO_LOOPS: usize = 16;

/// Maximum total inlining budget per caller function (total inlined instructions).
/// Prevents exponential blowup from recursive inlining chains.
/// Lowered from 800 to 350 for v2 to curb code-size bloat on user-space workloads
/// (see MAX_INLINE_INSTRUCTIONS note). Correctness-critical tiny/small/static and
/// always_inline inlining is governed by separate thresholds and is unaffected.
const MAX_INLINE_BUDGET_PER_CALLER: usize = 350;

/// Maximum total instruction count for a caller function after inlining.
/// When the caller exceeds this threshold, normal (non-always_inline) inlining
/// stops. This prevents stack frame bloat: in CCC's codegen model, each SSA
/// value gets a stack slot (~8 bytes), so a function with many instructions
/// can easily produce a multi-KB stack frame that overflows the kernel's 16KB
/// stack. GCC enforces similar limits via -fconserve-stack.
/// Set to 200 to keep stack frames under ~2KB even after optimization,
/// leaving headroom for callers higher on the call stack (kernel functions
/// like mm/page_alloc.c have 10+ level deep call chains that can easily
/// overflow the 16KB kernel stack if individual frames are too large).
const MAX_CALLER_INSTRUCTIONS_AFTER_INLINE: usize = 200;

/// Maximum instructions for __attribute__((always_inline)) functions.
/// GCC always inlines __attribute__((always_inline)) regardless of size, and
/// failing to do so can cause section mismatch errors in the kernel (e.g., when
/// an always_inline function in .text accesses __initconst data, but its caller
/// is in .init.text). Stack frame bloat from large inlined functions is handled
/// separately by MAX_CALLER_INSTRUCTIONS_AFTER_INLINE for normal inlining and
/// by the kernel's -fconserve-stack. Set high enough to cover all real always_inline
/// functions in the kernel (e.g., intel_pmu_init_hybrid at ~250 IR instructions).
const MAX_ALWAYS_INLINE_INSTRUCTIONS: usize = 500;

/// Maximum blocks for __attribute__((always_inline)) functions.
/// GCC has no block limit for always_inline — it always inlines them.
/// Large always_inline kernel functions like __mutex_lock_common in
/// kernel/locking/mutex.c can generate 215+ basic blocks from complex
/// control flow (multiple if/else chains, error handling). Set high
/// enough to handle all real kernel always_inline functions.
const MAX_ALWAYS_INLINE_BLOCKS: usize = 500;

/// Maximum instructions for a callee to be considered "tiny" and always inlined
/// regardless of caller size. Tiny functions like `static inline bool f(void) { return false; }`
/// must always be inlined because:
/// 1. They have negligible impact on code/stack size
/// 2. Not inlining them can cause linker errors (references to symbols that are
///    only needed in dead code paths, e.g., kernel's folio_test_large_rmappable()
///    returns false when CONFIG_TRANSPARENT_HUGEPAGE is disabled, making the call
///    to folio_undo_large_rmappable() dead — but if not inlined, the linker sees
///    the undefined reference)
/// 3. GCC always inlines these trivial static inline functions
const MAX_TINY_INLINE_INSTRUCTIONS: usize = 5;

/// Maximum instructions for a callee to be considered "small" and always inlined
/// regardless of caller size. Small functions like `static inline void f(x, flag) { if (flag) g(x); }`
/// have 2-3 blocks (from if/else) and ~10-20 instructions. They must be inlined
/// because not inlining them can cause linker errors when they contain conditional
/// calls to symbols that don't exist in the current build configuration. Example:
/// kernel's fscache_clear_page_bits() calls __fscache_clear_page_bits() conditionally,
/// but if CONFIG_FSCACHE is disabled, the latter is not compiled. GCC inlines the
/// wrapper, so the conditional call becomes part of the caller — no linker error.
/// Without inlining, the standalone static function has an undefined reference.
const MAX_SMALL_INLINE_INSTRUCTIONS: usize = 20;

/// Maximum blocks for a callee to be considered "small" (see above).
const MAX_SMALL_INLINE_BLOCKS: usize = 3;

/// Maximum instructions for a `static` (non-`inline`) function to be eligible
/// for inlining. GCC at -O2 inlines small static functions even without the
/// `inline` keyword. This is critical for correctness: if a static function
/// references an undefined symbol in a conditionally-compiled code path, GCC
/// eliminates the reference by inlining, but without inlining we get a linker
/// error. Example: kernel's pxp_fw_dependencies_completed() references
/// intel_pxp_gsccs_is_ready_for_sessions() which is not compiled in some configs.
const MAX_STATIC_NONINLINE_INSTRUCTIONS: usize = 30;

/// Maximum blocks for a `static` (non-`inline`) function to be eligible.
const MAX_STATIC_NONINLINE_BLOCKS: usize = 4;

/// Compile-time bounds for inlining a `static` (non-`inline`) function that
/// has exactly ONE direct call site in the whole module and whose address is
/// never taken anywhere (see `collect_value_referenced_functions`).
///
/// These are NOT size heuristics. A single-call-site static whose address is
/// not taken is DEAD after inlining its one site: the outlined body, its
/// prologue/epilogue, and the call sequence all disappear, so the net size
/// delta is a win essentially by construction (this is GCC's
/// `-finline-functions-called-once`, enabled at -O2+). The kernel's real-mode
/// boot corpus depends on it: `display_menu` (41 blocks, 98 instructions) and
/// `get_entry` (28 blocks) are kept outlined purely by the former 24-block /
/// 300-instruction caps while GCC folds them away and ends up 1.9 KB smaller
/// on arch/x86/boot/video.c alone.
///
/// The only true hazards are (a) unbounded compile time in huge translation
/// units (each such function is inlined at most once, so the work is linear,
/// but cloning a pathologically large body is still wasteful) and (b) stack
/// growth when the merged live set spills. Both are bounded below at levels
/// far above any sane function (the previous multi-site caps stay unchanged
/// for functions that SURVIVE inlining, where cloning is repeated size cost).
const MAX_SINGLE_CALL_SITE_STATIC_INSTRUCTIONS: usize = 4000;

/// Block counterpart of the compile-time bound above.
const MAX_SINGLE_CALL_SITE_STATIC_BLOCKS: usize = 800;

/// Cost ceiling for the two LOOP-NEST exemptions granted to single-call-site
/// statics (the -Os nested-loop veto and the loop_nest_merge caller-cap
/// veto).  The exemptions are otherwise unbounded because the callee dies at
/// its one site, but a large loop body merged into a hot outer loop can spill
/// more than the eliminated outline saves — measured on the zlib-ng Adler-32
/// corpus: `zlib_ng_adler32_c` costs 262 instructions after its own callees
/// expand, and inlining its final remaining site (after the cold validation
/// site folded) into the hot len-loop was a reproducible runtime regression.
/// The kernel boot corpus cases the exemption exists for all measure well
/// under this ceiling: display_menu 98, parse_earlyprintk 100, get_entry 41.
const MAX_SINGLE_SITE_LOOP_NEST_EXEMPTION_COST: usize = 160;

/// Budget for always_inline callees per caller. This budget is ONLY consumed
/// by true __attribute__((always_inline)) callees that exceed the "small"
/// threshold (> MAX_SMALL_INLINE_INSTRUCTIONS or > MAX_SMALL_INLINE_BLOCKS).
/// Non-always_inline callees use the separate budget_remaining, ensuring they
/// can never starve always_inline functions of budget. This separation prevents
/// section mismatch errors (e.g., idle_init → fork_idle) that occur when
/// always_inline functions fail to inline.
///
/// When the caller has a section attribute (e.g., .init.text), always_inline
/// callees bypass this budget entirely. This is critical for kernel __init
/// functions like intel_pmu_init that call hundreds of always_inline helpers
/// referencing .init.rodata — not inlining them causes modpost errors.
///
/// Small always_inline callees (≤ MAX_SMALL_INLINE_INSTRUCTIONS) don't consume
/// this budget because they have negligible impact and must be inlined for
/// linker correctness (inline asm "i" constraints, undefined symbols).
///
/// Set to 200 to keep stack frames from always_inline inlining under ~2KB.
/// CCC allocates ~8 bytes per SSA value, so 200 instructions add ~1.6KB to
/// the stack frame. Combined with the base function's frame, this typically
/// keeps total frame size under ~2KB, leaving headroom in the kernel's 16KB
/// stack for deep call chains (e.g., mm/page_alloc.c has 10+ levels).
/// Functions with section attributes bypass the budget entirely (to avoid
/// modpost errors). The standalone bodies of always_inline functions that
/// aren't fully inlined remain correct because __attribute__((error)) calls
/// are lowered as no-ops (not traps).
const MAX_ALWAYS_INLINE_BUDGET_PER_CALLER: usize = 200;
/// Dedicated budget for profile-guided force-inlined call sites (hot loop
/// sites inlined even into large callers). Bounded to keep PGO-driven growth
/// safe for stack-frame and I-cache size.
const MAX_PGO_FORCE_INLINE_BUDGET_PER_CALLER: usize = 600;

/// Additional always_inline budget for the second (correctness) pass.
/// After the main inlining loop exhausts max_rounds, any remaining
/// always_inline call sites are processed in a second pass with this
/// independent budget. This is separate from the main budget because
/// the second pass handles correctness-critical cases (e.g., KVM nVHE
/// functions where always_inline chains reference section-specific
/// symbols like __kvm_nvhe_gic_nonsecure_priorities that only exist
/// when inlined). Set to 400 to cover typical 2-3 level always_inline
/// chains (e.g., cpucap_is_possible has ~84 instructions and appears
/// 3+ times per function, totaling ~252 instructions). This limits
/// stack frame growth: the worst case adds ~400 * 8 = ~3.2KB to the
/// frame, but in practice many inlined instructions are eliminated by
/// constant folding and DCE.
const MAX_ALWAYS_INLINE_SECOND_PASS_BUDGET: usize = 400;

/// Maximum number of rounds for the second (correctness) pass.
/// Each round inlines one always_inline call site and re-scans.
/// Set high enough to handle functions with many always_inline call
/// sites that weren't reached in the main loop's max_rounds.
const MAX_ALWAYS_INLINE_SECOND_PASS_ROUNDS: usize = 300;

/// Hard cap on caller instruction count. When a caller exceeds this threshold,
/// even always_inline inlining is stopped (except for tiny callees that must be
/// inlined to avoid linker errors). This prevents kernel stack overflow from
/// deeply-nested always_inline chains (e.g., mm/page_alloc.c's __rmqueue ->
/// __rmqueue_smallest -> __rmqueue_fallback chain, all always_inline, which
/// can create functions with 1000+ instructions and 3KB+ stack frames that
/// overflow the kernel's 16KB stack when combined with deep call chains).
/// GCC can tolerate larger functions because its register allocator keeps most
/// values in registers; CCC's codegen spills every SSA value to the stack
/// (~8 bytes each), so we must be more conservative.
const MAX_CALLER_INSTRUCTIONS_HARD_CAP: usize = 500;

/// Absolute hard cap on caller instruction count for normal inlining.
/// When a caller exceeds this threshold, normal inlining stops — only
/// always_inline callees and tiny/small callees continue to be inlined.
///
/// This prevents catastrophic stack frame bloat in functions like the kernel's
/// shrink_folio_list (mm/vmscan.c), which calls hundreds of small inline
/// helpers. CCC's accumulator-based codegen creates one stack slot per SSA value,
/// so inlining many calls can produce thousands of multi-block values
/// with wide liveness intervals, creating 16KB+ stack frames that overflow the
/// kernel's 16KB stack.
///
/// always_inline callees are exempt because not inlining them violates C
/// semantics and causes section mismatch errors in the kernel (e.g., __init
/// callers referencing __initconst data through always_inline helpers that
/// end up as standalone .text functions). GCC always inlines them regardless
/// of caller size.
///
/// Tiny/small callees (≤ MAX_SMALL_INLINE_INSTRUCTIONS) are also exempt because:
/// 1. They have negligible impact on code/stack size
/// 2. Not inlining them can cause linker errors (conditional references to
///    undefined symbols, inline asm "i" constraints, section mismatches)
const MAX_CALLER_INSTRUCTIONS_ABSOLUTE_CAP: usize = 1000;

/// Maximum iterations when tracing IR value chains (Load->Store->Copy->GEP->...)
/// to resolve inline asm operands back to GlobalAddr or constant values.
const MAX_TRACE_CHAIN_LENGTH: usize = 20;

/// Maximum recursion depth for trace_operand_to_const when evaluating BinOp/Cmp
/// trees where both operands themselves need recursive tracing.
const MAX_TRACE_RECURSION_DEPTH: u32 = 10;

/// Select the best call site to inline from the given candidates.
///
/// Uses a two-pass strategy:
/// 1. First pass: pick tiny/small/static-inline callees (always inlined for
///    code correctness, e.g., constant-returning stubs, linker symbol resolution).
/// 2. Second pass: pick the first eligible normal callee that fits within
///    budget and caller size constraints.
///
/// Returns `(site, callee_inst_count, use_relaxed)` or `None` if no eligible site.
fn select_inline_site(
    call_sites: &[InlineCallSite],
    callee_map: &FxHashMap<String, CalleeData>,
    caller_too_large: bool,
    caller_at_hard_cap: bool,
    caller_at_absolute_cap: bool,
    caller_has_section: bool,
    caller_is_recursive: bool,
    budget_remaining: usize,
    always_inline_budget_remaining: usize,
    pgo_force_budget_remaining: usize,
    size_optimized: bool,
    loop_blocks: &FxHashSet<usize>,
    caller_has_loops: bool,
) -> Option<(InlineCallSite, usize, bool)> {
    // First pass: look for tiny/small callees anywhere in the function.
    // These are always inlined regardless of caller size because:
    // 1. They have negligible impact on code/stack size
    // 2. Not inlining them can cause linker errors from conditional
    //    references to undefined symbols (e.g., fscache_clear_page_bits)
    //
    // However, once the always_inline budget is exhausted, only TINY
    // callees (≤5 instructions, single block) and small __always_inline
    // callees are picked. Non-always_inline small callees (6-20 instructions)
    // individually have "negligible" impact but collectively cause catastrophic
    // stack bloat when a function has 200+ call sites (e.g., kernel's
    // shrink_folio_list). Small __always_inline callees are still inlined
    // because they have correctness requirements: inline asm "i" constraints
    // (e.g., arch_static_branch's __jump_table entries) need resolved symbol
    // references, and their standalone bodies emit invalid assembly (like
    // ".dword 0 - .") when the symbol can't be resolved.
    let budget_exhausted = always_inline_budget_remaining == 0;
    for site in call_sites {
        let callee_data = &callee_map[&site.callee_name];
        // Never use ordinary inlining for a recursive callee. A clone still
        // contains the recursive call, so fixed-point cloning only inflates
        // the caller and does not remove call overhead. Dedicated recursion
        // transforms run before this post-structural phase.
        if callee_data.is_recursive && !callee_data.is_always_inline {
            continue;
        }
        let callee_inst_count: usize = callee_data
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum();
        let is_tiny =
            callee_inst_count <= MAX_TINY_INLINE_INSTRUCTIONS && callee_data.blocks.len() <= 1;
        let is_small = callee_inst_count <= MAX_SMALL_INLINE_INSTRUCTIONS
            && callee_data.blocks.len() <= MAX_SMALL_INLINE_BLOCKS;
        // A seemingly tiny ordinary static wrapper can call a clonable loop
        // helper.  If the wrapper has several module-wide callers, selecting
        // it here duplicates that loop in every caller on later rounds.  The
        // source `spectral_norm` shape made this a 19% VM-screen regression;
        // retain the outlined wrapper so its loop owner is expanded once.
        // Attribute/section/PGO cases keep their stronger correctness or
        // profile-driven policy.
        if multisite_loop_wrapper_should_stay_outlined(
            callee_data,
            callee_inst_count,
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) || nested_loop_multisite_static_should_stay_outlined(
            callee_data,
            callee_inst_count,
            loop_blocks.contains(&site.block_idx),
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) || repeated_small_loop_clone_should_stay_outlined(
            callee_data,
            callee_inst_count,
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) {
            continue;
        }
        // Static inline functions that fit within normal limits should
        // always be inlined, matching GCC behavior. This is critical for
        // functions like ror32 (35 instructions) called from blake2s: without
        // inlining, shift amounts can't be constant-propagated, producing
        // massive unoptimized code with 28KB+ stack frames that overflow
        // the kernel's 16KB stack.
        let callee_block_limit = if callee_data.has_loops {
            MAX_INLINE_BLOCKS
        } else {
            MAX_INLINE_BLOCKS_NO_LOOPS
        };
        let is_static_inline_eligible = callee_data.is_static_inline
            && (callee_inst_count <= MAX_INLINE_INSTRUCTIONS
                || (callee_data.has_vector_intrinsics
                    && callee_inst_count <= MAX_VECTOR_STATIC_INLINE_INSTRUCTIONS))
            && callee_data.blocks.len()
                <= if callee_data.has_vector_intrinsics {
                    MAX_VECTOR_STATIC_INLINE_BLOCKS
                } else {
                    callee_block_limit
                };
        // For recursive callers, only inline tiny callees and always_inline callees.
        // Inlining larger callees into recursive functions multiplies the stack frame
        // increase by the recursion depth, easily causing stack overflow.
        // gnu_inline defs must also pass the recursive-caller guard: their
        // body is the ONLY definition in this TU, so a skipped site becomes
        // an undefined symbol in -nostdlib links (glibc rtld: the extern
        // inline `free` wrapper called from the RECURSIVE free_slotinfo).
        if caller_is_recursive
            && !is_tiny
            && !callee_data.is_always_inline
            && !callee_data.is_gnu_inline_def
        {
            continue;
        }
        // gnu_inline caps: generous enough for glibc's header bodies
        // (bsearch: ~35 insts, 8 blocks with a loop) but still bounded.
        let is_gnu_inline_eligible = callee_data.is_gnu_inline_def
            && callee_inst_count <= 128
            && callee_data.blocks.len() <= 16;
        if is_tiny
            || (is_small && (!budget_exhausted || callee_data.is_always_inline))
            || is_static_inline_eligible
            || is_gnu_inline_eligible
        {
            let use_relaxed = callee_data.is_always_inline || callee_data.exceeds_normal_limits;
            return Some((site.clone(), callee_inst_count, use_relaxed));
        }
    }

    // Second pass: use the first eligible normal callee.
    // Under -Os, admit normal/medium callees only when inlining cannot duplicate
    // a static body (one module-wide call site), when the call itself is in a
    // loop, or when a loop helper is called from another loop function.  The
    // first is normally a net size reduction; the loop cases preserve the most
    // valuable dynamic wins and avoid forcing hot loop state through memory at
    // helper boundaries, without cloning every medium helper at cold call sites.
    // Keep loop-containing callees in an enclosing loop below a strict size cap:
    // bounded hash-table loops benefit, while larger merged nests create severe
    // spills with the current register allocator (zlib-ng Adler-32 guards this).
    for site in call_sites {
        let callee_data = &callee_map[&site.callee_name];
        let call_is_in_loop = loop_blocks.contains(&site.block_idx);
        let callee_inst_count: usize = callee_data
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum();
        if multisite_loop_wrapper_should_stay_outlined(
            callee_data,
            callee_inst_count,
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) || nested_loop_multisite_static_should_stay_outlined(
            callee_data,
            callee_inst_count,
            loop_blocks.contains(&site.block_idx),
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) || repeated_small_loop_clone_should_stay_outlined(
            callee_data,
            callee_inst_count,
            caller_has_section,
            site.pgo_force,
            size_optimized,
        ) {
            continue;
        }
        // Cost loop-body cloning after accounting for tiny/static-inline
        // descendants that the fixed-point inliner will necessarily expand
        // in the caller.  The raw snapshot can be misleadingly small: Linux's
        // a20_test is 24 instructions before seven inline-asm wrappers expand,
        // but 66 instructions afterwards.  Using only 24 bypassed the 64-
        // instruction nested-loop guard and cloned that expanded body twice.
        let size_inline_cost = callee_data.size_inline_cost;
        if size_optimized
            && !callee_data.is_always_inline
            // Single-call-site statics are exempt up to the measured loop-
            // nest ceiling: their outlined body disappears at this one site,
            // so cloning it into a loop nest still shrinks total size (the
            // veto exists for callees whose bodies survive and are re-cloned
            // per site).  Above the ceiling the merge's spill cost can
            // dominate (see MAX_SINGLE_SITE_LOOP_NEST_EXEMPTION_COST).
            && !(callee_data.is_single_call_site_static
                && size_inline_cost <= MAX_SINGLE_SITE_LOOP_NEST_EXEMPTION_COST)
            && call_is_in_loop
            && callee_data.has_loops
            && size_inline_cost > MAX_SIZE_NESTED_LOOP_INLINE_INSTRUCTIONS
        {
            continue;
        }
        if size_optimized
            && !callee_data.is_always_inline
            && !callee_data.is_single_call_site_static
            && !(call_is_in_loop
                && (!callee_data.has_loops
                    || size_inline_cost <= MAX_SIZE_NESTED_LOOP_INLINE_INSTRUCTIONS))
            && !(callee_data.has_loops && caller_has_loops && !call_is_in_loop)
        {
            continue;
        }
        if callee_data.is_recursive && !callee_data.is_always_inline {
            continue;
        }
        // For recursive callers, skip non-tiny, non-always_inline callees.
        // (Tiny callees were handled in the first pass.)
        if caller_is_recursive && !callee_data.is_always_inline {
            continue;
        }
        // A profile-guided FORCE-INLINE site is hot (in a hot loop /
        // high frequency); it should be inlined even when the caller is large
        // or the normal budget is tight — the PGO advantage. We bypass the
        // caller-size / cap gates for it, but still bound total PGO-driven
        // growth with a dedicated budget so we never unboundedly bloat a
        // caller (stack-frame and I-cache safety).
        if !site.pgo_force {
            // When the caller has a section attribute (e.g., .init.text),
            // allow inlining small callees even into large callers to
            // prevent section mismatch errors.
            // Single-call-site static callees are exempt from the soft
            // caller-size cap: the callee is dead after inlining (net code
            // shrink), so growing the caller does not increase total code
            // size. The hard and absolute caps below still apply as safety
            // brakes.
            //
            // Call sites inside loops are also exempt (up to the normal
            // callee size limit): a call in a hot loop pays its overhead
            // every iteration, so inlining it is almost always a win even in
            // a large caller.
            //
            // Exception: inlining a loop-containing callee into a caller that
            // already has loops merges two hot loop nests into one function,
            // and the combined register pressure forces the loop variables to
            // spill. That veto is for callees that SURVIVE the inline (their
            // body is cloned per site, so spills are pure loss). A
            // single-call-site static whose address is not taken dies at its
            // one site: the outlined body, prologue/epilogue and call sequence
            // all disappear, which dominates bounded spill growth in the
            // merged nest — GCC applies the same reasoning via
            // -finline-functions-called-once. Keep such callees exempt, up
            // to the same measured loop-nest ceiling as the -Os veto.
            let in_loop = loop_blocks.contains(&site.block_idx);
            let loop_nest_merge = callee_data.has_loops
                && caller_has_loops
                && callee_inst_count > MAX_SMALL_INLINE_INSTRUCTIONS;
            let size_cap_exempt = callee_data.is_single_call_site_static
                && callee_data.size_inline_cost <= MAX_SINGLE_SITE_LOOP_NEST_EXEMPTION_COST;
            if caller_too_large
                && !callee_data.is_always_inline
                && !size_cap_exempt
                && !(in_loop
                    && !callee_data.has_loops
                    && callee_inst_count <= MAX_INLINE_INSTRUCTIONS)
                && (!caller_has_section || callee_inst_count > MAX_SMALL_INLINE_INSTRUCTIONS)
            {
                continue;
            }
            // Absolute cap: stop normal inlining for extremely large callers.
            // always_inline callees MUST still be inlined (C semantic requirement).
            if caller_at_absolute_cap && !callee_data.is_always_inline {
                continue;
            }
            // Hard cap: stop normal inlining to prevent kernel stack overflow.
            // always_inline callees are still inlined (C semantic requirement),
            // but are limited by the always_inline budget.
            if caller_at_hard_cap && !callee_data.is_always_inline {
                continue;
            }
        }
        let use_relaxed = callee_data.is_always_inline || callee_data.exceeds_normal_limits;
        // Budget enforcement: always_inline callees use a separate budget;
        // non-always_inline callees use the normal budget. PGO-forced sites
        // use the dedicated (larger) PGO budget.
        if callee_data.is_always_inline {
            // When the caller has a section attribute, always_inline callees
            // bypass the budget entirely (critical for kernel init functions
            // like intel_pmu_init that call hundreds of always_inline helpers).
            if !caller_has_section {
                let is_tiny = callee_inst_count <= MAX_TINY_INLINE_INSTRUCTIONS
                    && callee_data.blocks.len() <= 1;
                if !is_tiny && callee_inst_count > always_inline_budget_remaining {
                    continue;
                }
            }
        } else if site.pgo_force {
            if callee_inst_count > pgo_force_budget_remaining {
                continue;
            }
        } else if !callee_data.is_single_call_site_static && callee_inst_count > budget_remaining {
            // Single-call-site statics don't consume the normal budget: each
            // is inlined at most once module-wide, so they cannot cause the
            // exponential blowup the budget guards against.
            continue;
        }
        return Some((site.clone(), callee_inst_count, use_relaxed));
    }

    None
}

/// Run the inlining pass on the module.
/// Returns the number of call sites inlined.
pub fn run(module: &mut IrModule) -> usize {
    inline_run(module, false)
}

/// -Os/-Oz entry point. Tiny/small callees still inline; normal static
/// callees are admitted only by the size-aware and loop-aware profitability
/// gates in `select_inline_site`. Skipping all inlining made LCCC's -Os
/// binaries larger than -O3 (for example, standalone zlib-ng helpers).
pub fn run_size_optimized(module: &mut IrModule) -> usize {
    inline_run(module, true)
}

fn inline_run(module: &mut IrModule, size_optimized: bool) -> usize {
    inline_run_impl(module, size_optimized, false)
}

/// `-O0` entry point: GCC semantics require `__attribute__((always_inline))`
/// functions to be inlined at EVERY optimization level — the attribute is a
/// correctness constraint (gnu_inline wrappers have no out-of-line body, so
/// an un-inlined call is a hard undefined-symbol link error), not an
/// optimization choice.  This tier admits ONLY always_inline callees and
/// leaves every other body untouched, preserving -O0's no-optimizer contract.
pub fn run_always_inline_only(module: &mut IrModule) -> usize {
    inline_run_impl(module, false, true)
}

fn inline_run_impl(module: &mut IrModule, size_optimized: bool, always_inline_only: bool) -> usize {
    let mut total_inlined = 0;
    let debug_inline = std::env::var("CCC_INLINE_DEBUG").is_ok();
    let skip_list: Vec<String> = std::env::var("CCC_INLINE_SKIP")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string())
        .collect();

    // Build a snapshot of eligible callees (we can't borrow module mutably while reading callees).
    // We clone the callee function bodies since we need them while mutating callers.
    let mut callee_map = build_callee_map(module);
    if always_inline_only {
        callee_map.retain(|_, data| data.is_always_inline);
    }

    if callee_map.is_empty() {
        return 0;
    }

    // PGO: check for profile to guide inlining
    let pgo_profile = crate::pgo::get_pgo_profile();
    if pgo_profile.is_some() {
        eprintln!(
            "lccc: PGO inline: using profile with {} functions",
            pgo_profile.unwrap().functions.len()
        );
    }

    if debug_inline {
        eprintln!(
            "[INLINE] Callee map has {} eligible functions:",
            callee_map.len()
        );
        for (name, data) in &callee_map {
            let ic: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
            eprintln!(
                "[INLINE]   '{}': {} blocks, {} instructions, {} params",
                name,
                data.blocks.len(),
                ic,
                data.num_params
            );
        }
    }

    // Compute the module-global max block ID. Block labels (.LBB{id}) are global in the
    // assembly output, so inlined blocks must use IDs that don't collide with ANY
    // function's block IDs, not just the caller's.
    let mut global_max_block_id: u32 = 0;
    for func in &module.functions {
        for block in &func.blocks {
            if block.label.0 > global_max_block_id {
                global_max_block_id = block.label.0;
            }
        }
    }

    // Process each function as a potential caller
    for func_idx in 0..module.functions.len() {
        if module.functions[func_idx].is_declaration {
            continue;
        }

        let caller_has_section = module.functions[func_idx].section.is_some();
        // Check if the caller is directly recursive (calls itself).
        // If so, we must be very conservative about inlining other callees
        // into it, because each inlined callee increases the stack frame
        // that gets multiplied by the recursion depth.
        let caller_is_recursive = {
            let func = &module.functions[func_idx];
            func.blocks.iter().any(|block| {
                block.instructions.iter().any(|inst| {
                    if let Instruction::Call {
                        func: callee_name, ..
                    } = inst
                    {
                        callee_name == &func.name
                    } else {
                        false
                    }
                })
            })
        };
        let mut budget_remaining = MAX_INLINE_BUDGET_PER_CALLER;
        let mut always_inline_budget_remaining = MAX_ALWAYS_INLINE_BUDGET_PER_CALLER;
        let mut pgo_force_budget_remaining = MAX_PGO_FORCE_INLINE_BUDGET_PER_CALLER;
        // During one -Os inliner invocation, clone a large ordinary callee at
        // most once into a given caller. This permits one profitable
        // specialization while avoiding repeated medium-body expansion.
        let mut size_inlined_large_callees: FxHashSet<String> = FxHashSet::default();
        // Iterate to handle chains of inlined calls (A calls B calls C, all small inline).
        // Limit iterations to prevent infinite loops from recursive inline functions.
        let max_rounds = 200;
        for _round in 0..max_rounds {
            // Check if the caller has grown too large for further normal inlining.
            // Each SSA value in CCC gets an 8-byte stack slot, so functions with
            // too many instructions will have massive stack frames. Stop normal
            // inlining once the caller exceeds the threshold; always_inline
            // callees are still inlined (required by C semantics).
            let caller_inst_count: usize = module.functions[func_idx]
                .blocks
                .iter()
                .map(|b| b.instructions.len())
                .sum();
            let caller_too_large = caller_inst_count > MAX_CALLER_INSTRUCTIONS_AFTER_INLINE;
            let caller_at_hard_cap = caller_inst_count > MAX_CALLER_INSTRUCTIONS_HARD_CAP;
            let caller_at_absolute_cap = caller_inst_count > MAX_CALLER_INSTRUCTIONS_ABSOLUTE_CAP;

            // Find call sites to inline in the current function.
            // When the caller has a custom section attribute, also consider callees
            // that exceed normal limits, to avoid dangerous cross-section calls.
            let mut call_sites = find_inline_call_sites(
                &module.functions[func_idx],
                &callee_map,
                &skip_list,
                caller_has_section,
            );
            // PGO: filter/adjust call sites based on profile. On a FLAT
            // profile (no hot/cold spread) PGO inlining has no informative
            // signal; skip reading the profile entirely so inlining is
            // byte-identical to plain -O2 (prevents pass perturbation that can
            // regress hot paths — see inline_pgo::inline_decisions_active).
            if let Some(profile) = crate::pgo::get_pgo_profile() {
                if crate::pgo::inline_pgo::inline_decisions_active() {
                    let caller_name = module.functions[func_idx].name.clone();
                    // Per-call-site hotness. Derive block counts for the
                    // CURRENT caller CFG from the h0-keyed profile (the CFG may
                    // have changed since training; derive handles drift
                    // gracefully) so the decision sees the count of the block
                    // containing each call (LLVM isHotCallSite/BFI).
                    let caller_fn = &module.functions[func_idx];
                    let mut caller_fp = crate::pgo::prepass_profile(&caller_name);
                    if let Some(fp) = caller_fp.as_mut() {
                        crate::pgo::profile::derive_block_counts(caller_fn, fp);
                    }
                    // For each call site, check if PGO says to force inline or deny
                    let mut filtered = Vec::with_capacity(16);
                    for site in call_sites {
                        if let Some(callee) = callee_map.get(&site.callee_name) {
                            // Get callee name
                            let callee_name = &site.callee_name;
                            let site_cnt = caller_fp
                                .as_ref()
                                .map(|fp| {
                                    let lbl =
                                        module.functions[func_idx].blocks[site.block_idx].label;
                                    fp.block_count(lbl)
                                })
                                .unwrap_or(0);
                            // Check PGO inline decision
                            if let Some(force) = crate::pgo::inline_pgo::should_inline_site(
                                &module.functions[func_idx],
                                &{
                                    // Reconstruct IrFunction for callee from callee_map data
                                    // callee_map stores blocks, we need to create a dummy IrFunction
                                    let mut dummy = crate::ir::reexports::IrFunction {
                                        name: callee_name.clone(),
                                        return_type: crate::common::types::IrType::I32,
                                        params: vec![],
                                        blocks: callee.blocks.clone(),
                                        is_variadic: false,
                                        is_declaration: false,
                                        is_static: false,
                                        is_inline: false,
                                        is_always_inline: false,
                                        is_noinline: false,
                                        next_value_id: 0,
                                        fp_expr_tags: Default::default(),
                                        next_label: 0,
                                        section: None,
                                        visibility: None,
                                        is_weak: false,
                                        is_used: false,
                                        has_inlined_calls: false,
                                        param_alloca_values: vec![],
                                        uses_sret: false,
                                        is_fastcall: false,
                                        is_naked: false,
                                        no_instrument: false,
                                        global_init_label_blocks: vec![],
                                        ret_eightbyte_classes: vec![],
                                        ret_is_f128_sse: false,
                                        is_gnu_inline_def: false,
                                        loop_promoted_f64_values: Vec::new(),
                                    };
                                    dummy
                                },
                                site_cnt,
                                Some(profile),
                            ) {
                                if force {
                                    let mut s2 = site;
                                    s2.pgo_force = true;
                                    filtered.push(s2);
                                    continue;
                                } else {
                                    continue;
                                } // deny
                            }
                            // Also check threshold multiplier
                            let mult = crate::pgo::inline_pgo::inline_threshold_multiplier(
                                &caller_name,
                                callee_name,
                                profile,
                            );
                            if mult < 0.4 {
                                // Cold: skip unless tiny
                                let inst_count: usize =
                                    callee.blocks.iter().map(|b| b.instructions.len()).sum();
                                if inst_count > 5 {
                                    continue;
                                }
                            }
                        }
                        filtered.push(site);
                    }
                    call_sites = filtered;
                } // end inline_decisions_active
            }
            if size_optimized && !size_inlined_large_callees.is_empty() {
                call_sites.retain(|site| !size_inlined_large_callees.contains(&site.callee_name));
            }
            if call_sites.is_empty() {
                break;
            }

            // Select best call site (prioritizes tiny/small, respects budgets).
            // Compute which blocks are inside loops: call sites there are
            // high-value (per-iteration call overhead) and exempt from the
            // caller-size caps. Also used to avoid inlining loop-containing
            // callees into a caller that already has loops.
            let needs_loop_info = size_optimized
                || caller_too_large
                || call_sites
                    .iter()
                    .any(|s| callee_map[&s.callee_name].has_loops);
            let loop_blocks: FxHashSet<usize> = if needs_loop_info {
                let cfg = crate::ir::analysis::CfgAnalysis::build(&module.functions[func_idx]);
                let loops = loop_analysis::find_natural_loops(
                    cfg.num_blocks,
                    &cfg.preds,
                    &cfg.succs,
                    &cfg.idom,
                );
                let mut s = FxHashSet::default();
                for lp in &loops {
                    s.extend(lp.body.iter().copied());
                }
                s
            } else {
                FxHashSet::default()
            };
            let caller_has_loops = !loop_blocks.is_empty();
            let found_site = select_inline_site(
                &call_sites,
                &callee_map,
                caller_too_large,
                caller_at_hard_cap,
                caller_at_absolute_cap,
                caller_has_section,
                caller_is_recursive,
                budget_remaining,
                always_inline_budget_remaining,
                pgo_force_budget_remaining,
                size_optimized,
                &loop_blocks,
                caller_has_loops,
            );
            let (site, callee_inst_count, _use_relaxed) = match found_site {
                Some(s) => s,
                None => {
                    if debug_inline && caller_too_large {
                        eprintln!(
                            "[INLINE] No more always_inline callees to inline into '{}' (caller has {} instructions)",
                            module.functions[func_idx].name, caller_inst_count
                        );
                    }
                    break;
                }
            };
            let callee_data = &callee_map[&site.callee_name];

            let success = inline_call_site(
                &mut module.functions[func_idx],
                &site,
                callee_data,
                &mut global_max_block_id,
            );

            if success {
                if size_optimized
                    && !callee_data.is_always_inline
                    && !callee_data.is_single_call_site_static
                    && callee_inst_count > MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS
                {
                    size_inlined_large_callees.insert(site.callee_name.clone());
                }
                if debug_inline {
                    eprintln!(
                        "[INLINE] Inlined '{}' into '{}'",
                        site.callee_name, module.functions[func_idx].name
                    );
                }
                if std::env::var("CCC_INLINE_VALIDATE").is_ok() {
                    validate_function_values(&module.functions[func_idx], &site.callee_name);
                }
                if std::env::var("CCC_INLINE_DUMP_IR").is_ok() {
                    dump_function_ir(
                        &module.functions[func_idx],
                        &format!(
                            "after inlining '{}' into '{}'",
                            site.callee_name, module.functions[func_idx].name
                        ),
                    );
                }
                // Deduct from the always_inline budget only when the callee
                // is actually always_inline. Non-always_inline callees that
                // use the relaxed path (exceeds_normal_limits) should not
                // consume the always_inline budget — otherwise a large
                // exceeds_normal_limits callee (e.g., find_next_bit with 77
                // instructions) can exhaust the budget and prevent true
                // always_inline callees (e.g., idle_init) from being inlined,
                // causing section mismatch errors.
                let callee_is_always_inline = callee_map
                    .get(&site.callee_name)
                    .map(|d| d.is_always_inline)
                    .unwrap_or(false);
                if callee_is_always_inline {
                    // Don't deduct small callees from the always_inline budget.
                    // Small callees (≤20 instructions) have negligible individual
                    // impact and are always inlined regardless of budget (handled
                    // in the first pass). Not counting them preserves budget for
                    // larger always_inline callees that actually matter (e.g.,
                    // intel_pmu_init_glc at 211 instructions needs to be inlined
                    // into intel_pmu_init to avoid section mismatches).
                    let callee_blocks = callee_map
                        .get(&site.callee_name)
                        .map(|d| d.blocks.len())
                        .unwrap_or(0);
                    let is_small = callee_inst_count <= MAX_SMALL_INLINE_INSTRUCTIONS
                        && callee_blocks <= MAX_SMALL_INLINE_BLOCKS;
                    if !is_small {
                        always_inline_budget_remaining =
                            always_inline_budget_remaining.saturating_sub(callee_inst_count);
                    }
                } else if site.pgo_force {
                    // PGO-forced sites consume the dedicated PGO budget.
                    pgo_force_budget_remaining =
                        pgo_force_budget_remaining.saturating_sub(callee_inst_count);
                } else {
                    budget_remaining = budget_remaining.saturating_sub(callee_inst_count);
                }
                total_inlined += 1;
                module.functions[func_idx].has_inlined_calls = true;
            } else {
                break;
            }
        }

        // Second pass: ensure all remaining __always_inline call sites are inlined.
        // The main loop above may exhaust max_rounds before processing all call sites
        // in functions with 200+ inline sites (e.g., kernel's ___slab_alloc in mm/slub.c).
        // When always_inline functions like cpucap_is_possible() are left un-inlined,
        // their standalone bodies contain BRK traps from unresolved compiletime_assert,
        // causing kernel crashes. This pass also handles cases where the main loop's
        // budget was exhausted, leaving correctness-critical always_inline chains
        // (e.g., KVM nVHE functions referencing section-specific symbols) un-inlined.
        //
        // This pass uses its own independent budget (not shared with the main loop)
        // to allow small always_inline chains needed for correctness while preventing
        // large chains from causing stack overflow. Combined with the main loop, the
        // worst case is 200 + 400 = 600 always_inline instructions per caller.
        //
        // Note: this pass intentionally does NOT check caller_too_large /
        // caller_at_hard_cap / caller_at_absolute_cap. These are correctness-
        // critical inlines (avoiding linker errors and BRK crashes), so they
        // must proceed regardless of caller size. The budget limit (400 inst)
        // provides the growth bound instead.
        let mut second_pass_budget = MAX_ALWAYS_INLINE_SECOND_PASS_BUDGET;
        for _round in 0..MAX_ALWAYS_INLINE_SECOND_PASS_ROUNDS {
            let mut call_sites = find_inline_call_sites(
                &module.functions[func_idx],
                &callee_map,
                &skip_list,
                caller_has_section,
            );
            // PGO: filter/adjust call sites based on profile. On a FLAT
            // profile (no hot/cold spread) PGO inlining has no informative
            // signal; skip reading the profile entirely so inlining is
            // byte-identical to plain -O2 (prevents pass perturbation that can
            // regress hot paths — see inline_pgo::inline_decisions_active).
            if let Some(profile) = crate::pgo::get_pgo_profile() {
                if crate::pgo::inline_pgo::inline_decisions_active() {
                    let caller_name = module.functions[func_idx].name.clone();
                    // Per-call-site hotness. Derive block counts for the
                    // CURRENT caller CFG from the h0-keyed profile (the CFG may
                    // have changed since training; derive handles drift
                    // gracefully) so the decision sees the count of the block
                    // containing each call (LLVM isHotCallSite/BFI).
                    let caller_fn = &module.functions[func_idx];
                    let mut caller_fp = crate::pgo::prepass_profile(&caller_name);
                    if let Some(fp) = caller_fp.as_mut() {
                        crate::pgo::profile::derive_block_counts(caller_fn, fp);
                    }
                    // For each call site, check if PGO says to force inline or deny
                    let mut filtered = Vec::with_capacity(16);
                    for site in call_sites {
                        if let Some(callee) = callee_map.get(&site.callee_name) {
                            // Get callee name
                            let callee_name = &site.callee_name;
                            let site_cnt = caller_fp
                                .as_ref()
                                .map(|fp| {
                                    let lbl =
                                        module.functions[func_idx].blocks[site.block_idx].label;
                                    fp.block_count(lbl)
                                })
                                .unwrap_or(0);
                            // Check PGO inline decision
                            if let Some(force) = crate::pgo::inline_pgo::should_inline_site(
                                &module.functions[func_idx],
                                &{
                                    // Reconstruct IrFunction for callee from callee_map data
                                    // callee_map stores blocks, we need to create a dummy IrFunction
                                    let mut dummy = crate::ir::reexports::IrFunction {
                                        name: callee_name.clone(),
                                        return_type: crate::common::types::IrType::I32,
                                        params: vec![],
                                        blocks: callee.blocks.clone(),
                                        is_variadic: false,
                                        is_declaration: false,
                                        is_static: false,
                                        is_inline: false,
                                        is_always_inline: false,
                                        is_noinline: false,
                                        next_value_id: 0,
                                        fp_expr_tags: Default::default(),
                                        next_label: 0,
                                        section: None,
                                        visibility: None,
                                        is_weak: false,
                                        is_used: false,
                                        has_inlined_calls: false,
                                        param_alloca_values: vec![],
                                        uses_sret: false,
                                        is_fastcall: false,
                                        is_naked: false,
                                        no_instrument: false,
                                        global_init_label_blocks: vec![],
                                        ret_eightbyte_classes: vec![],
                                        ret_is_f128_sse: false,
                                        is_gnu_inline_def: false,
                                        loop_promoted_f64_values: Vec::new(),
                                    };
                                    dummy
                                },
                                site_cnt,
                                Some(profile),
                            ) {
                                if force {
                                    let mut s2 = site;
                                    s2.pgo_force = true;
                                    filtered.push(s2);
                                    continue;
                                } else {
                                    continue;
                                } // deny
                            }
                            // Also check threshold multiplier
                            let mult = crate::pgo::inline_pgo::inline_threshold_multiplier(
                                &caller_name,
                                callee_name,
                                profile,
                            );
                            if mult < 0.4 {
                                // Cold: skip unless tiny
                                let inst_count: usize =
                                    callee.blocks.iter().map(|b| b.instructions.len()).sum();
                                if inst_count > 5 {
                                    continue;
                                }
                            }
                        }
                        filtered.push(site);
                    }
                    call_sites = filtered;
                } // end inline_decisions_active
            }
            if call_sites.is_empty() {
                break;
            }

            // Only look for always_inline callees
            let mut found = false;
            for site in &call_sites {
                let callee_data = &callee_map[&site.callee_name];
                if !callee_data.is_always_inline {
                    continue;
                }
                let callee_inst_count: usize = callee_data
                    .blocks
                    .iter()
                    .map(|b| b.instructions.len())
                    .sum();
                let is_tiny = callee_inst_count <= MAX_TINY_INLINE_INSTRUCTIONS
                    && callee_data.blocks.len() <= 1;
                let is_small = callee_inst_count <= MAX_SMALL_INLINE_INSTRUCTIONS
                    && callee_data.blocks.len() <= MAX_SMALL_INLINE_BLOCKS;
                // Tiny and small always_inline callees always pass; others must fit
                // in the second pass budget. Small always_inline callees bypass the
                // budget because they have correctness requirements (e.g., inline asm
                // "i" constraints in arch_static_branch). Callers with section
                // attributes bypass budget (section-specific symbols like __kvm_nvhe_*
                // MUST be resolved through inlining).
                if !is_tiny
                    && !is_small
                    && !caller_has_section
                    && callee_inst_count > second_pass_budget
                {
                    continue;
                }
                let success = inline_call_site(
                    &mut module.functions[func_idx],
                    site,
                    callee_data,
                    &mut global_max_block_id,
                );
                if success {
                    if debug_inline {
                        eprintln!(
                            "[INLINE] Inlined always_inline '{}' into '{}' (second pass)",
                            site.callee_name, module.functions[func_idx].name
                        );
                    }
                    if std::env::var("CCC_INLINE_VALIDATE").is_ok() {
                        validate_function_values(&module.functions[func_idx], &site.callee_name);
                    }
                    if std::env::var("CCC_INLINE_DUMP_IR").is_ok() {
                        dump_function_ir(
                            &module.functions[func_idx],
                            &format!(
                                "after inlining '{}' into '{}' (second pass)",
                                site.callee_name, module.functions[func_idx].name
                            ),
                        );
                    }
                    total_inlined += 1;
                    module.functions[func_idx].has_inlined_calls = true;
                    if !is_tiny && !is_small && !caller_has_section {
                        second_pass_budget = second_pass_budget.saturating_sub(callee_inst_count);
                    }
                    found = true;
                    break; // Re-scan after each inline
                }
            }
            if !found {
                break;
            }
        }
    }

    // After ALL inlining is complete, resolve input_symbols for InlineAsm instructions.
    // This must run after the entire inlining pass because multi-level inline chains
    // (e.g., arch_static_branch → static_key_false → trace_tlb_flush) need all levels
    // to be inlined before we can trace values back to their original GlobalAddr/Const.
    // Running resolution per-function would fail for intermediate functions whose
    // parameters haven't been replaced with concrete values yet.
    for func_idx in 0..module.functions.len() {
        if module.functions[func_idx].has_inlined_calls {
            resolve_inline_asm_symbols(&mut module.functions[func_idx]);
        }
    }

    total_inlined
}

/// After inlining, resolve input_symbols for InlineAsm instructions by tracing
/// Value operands back to their definitions. When an always_inline function
/// containing asm goto with "i" constraints is inlined, the constraint operands
/// become IR Values (loaded from parameter allocas) rather than compile-time constants.
/// This function traces those values back through Load/Copy/Store chains to find
/// the original GlobalAddr instruction, recovering the symbol name.
///
/// Without this, the backend sees an "unsatisfiable immediate" and skips the
/// entire asm body (including .pushsection __jump_table entries), breaking the
/// kernel's static branch mechanism and causing boot failures.
fn resolve_inline_asm_symbols(func: &mut IrFunction) {
    // Build a map from Value -> defining instruction for the whole function.
    // We store the instruction itself (cloned) for lookup.
    let mut value_defs: FxHashMap<u32, Instruction> = FxHashMap::default();
    // Also track Store instructions: alloca_ptr -> stored value
    // This lets us trace: Load(alloca) -> Store(val, alloca) -> val
    let mut alloca_stores: FxHashMap<u32, Operand> = FxHashMap::default();

    for block in func.blocks.iter() {
        for inst in &block.instructions {
            // Record value definitions
            if let Some(v) = inst.dest() {
                value_defs.insert(v.0, inst.clone());
            }
            // Record stores to alloca pointers (the last store wins; for inlined
            // parameter allocas there's typically exactly one store at the top)
            if let Instruction::Store { val, ptr, .. } = inst {
                alloca_stores.insert(ptr.0, *val);
            }
        }
    }

    // Helper: trace a Value back to find a GlobalAddr name + accumulated offset.
    let trace_to_global = |start_val: u32| -> Option<String> {
        trace_value_to_global(start_val, &value_defs, &alloca_stores)
    };

    // Helper: trace a Value or Const operand to find a constant integer value.
    // Delegates to the standalone recursive function.
    let trace_to_const = |op: &Operand| -> Option<i64> {
        trace_operand_to_const(op, &value_defs, &alloca_stores, 0)
    };

    // Now scan all blocks for InlineAsm instructions and fix up input_symbols
    let debug_resolve = std::env::var("CCC_INLINE_DEBUG").is_ok();
    for block in func.blocks.iter_mut() {
        for inst in block.instructions.iter_mut() {
            if let Instruction::InlineAsm {
                inputs,
                input_symbols,
                template,
                ..
            } = inst
            {
                if debug_resolve && template.contains(".pushsection") {
                    eprintln!(
                        "[RESOLVE_ASM] Found InlineAsm with .pushsection in func '{}'",
                        func.name
                    );
                    eprintln!(
                        "[RESOLVE_ASM]   inputs: {:?}",
                        inputs
                            .iter()
                            .map(|(c, o, n)| (c.clone(), format!("{:?}", o), n.clone()))
                            .collect::<Vec<_>>()
                    );
                    eprintln!("[RESOLVE_ASM]   input_symbols: {:?}", input_symbols);
                }
                let num_outputs_in_sym = if input_symbols.len() > inputs.len() {
                    input_symbols.len() - inputs.len()
                } else {
                    0
                };
                for (i, (constraint, operand, _name)) in inputs.iter_mut().enumerate() {
                    let sym_idx = num_outputs_in_sym + i;
                    if sym_idx >= input_symbols.len() {
                        if debug_resolve {
                            eprintln!(
                                "[RESOLVE_ASM]   input[{}]: sym_idx {} >= input_symbols.len() {}, skip",
                                i,
                                sym_idx,
                                input_symbols.len()
                            );
                        }
                        continue;
                    }
                    // Only fix up entries that are currently None
                    if input_symbols[sym_idx].is_some() {
                        if debug_resolve {
                            eprintln!(
                                "[RESOLVE_ASM]   input[{}]: already has symbol {:?}, skip",
                                i, input_symbols[sym_idx]
                            );
                        }
                        continue;
                    }
                    // Only care about immediate-only constraints ("i", "n", etc.)
                    if !constraint_is_immediate_only(constraint) {
                        if debug_resolve {
                            eprintln!(
                                "[RESOLVE_ASM]   input[{}]: constraint '{}' not imm-only, skip",
                                i, constraint
                            );
                        }
                        continue;
                    }
                    if debug_resolve {
                        eprintln!(
                            "[RESOLVE_ASM]   input[{}]: constraint '{}', operand={:?}",
                            i, constraint, operand
                        );
                    }
                    match operand {
                        Operand::Value(v) => {
                            // Try to trace this value back to a GlobalAddr
                            if let Some(sym_name) = trace_to_global(v.0) {
                                if debug_resolve {
                                    eprintln!(
                                        "[RESOLVE_ASM]   -> resolved to symbol '{}'",
                                        sym_name
                                    );
                                }
                                input_symbols[sym_idx] = Some(sym_name);
                            }
                            // Also try to resolve to a constant and convert the operand
                            else if let Some(const_val) = trace_to_const(&Operand::Value(*v)) {
                                if debug_resolve {
                                    eprintln!("[RESOLVE_ASM]   -> resolved to const {}", const_val);
                                }
                                *operand = Operand::Const(IrConst::I64(const_val));
                            } else if debug_resolve {
                                eprintln!("[RESOLVE_ASM]   -> FAILED to resolve Value({})", v.0);
                            }
                        }
                        Operand::Const(_) => {
                            // Already a constant - nothing to fix
                        }
                    }
                }
            }
        }
    }
}

/// Trace a Value back to find a GlobalAddr name + accumulated offset.
/// Follow chains of: Copy(src) -> trace src, Load(ptr) -> Store(val, ptr) -> trace val
/// GEP offsets are accumulated so e.g. GEP(GlobalAddr("foo"), 8) yields "foo+8".
/// GEP with Value offsets are resolved via trace_operand_to_const.
fn trace_value_to_global(
    start_val: u32,
    value_defs: &FxHashMap<u32, Instruction>,
    alloca_stores: &FxHashMap<u32, Operand>,
) -> Option<String> {
    let mut current = start_val;
    let mut accumulated_offset: i64 = 0;
    for _ in 0..MAX_TRACE_CHAIN_LENGTH {
        if let Some(inst) = value_defs.get(&current) {
            match inst {
                Instruction::GlobalAddr { name, .. } => {
                    if accumulated_offset > 0 {
                        return Some(format!("{}+{}", name, accumulated_offset));
                    } else if accumulated_offset < 0 {
                        return Some(format!("{}{}", name, accumulated_offset));
                    }
                    return Some(name.clone());
                }
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } => {
                    current = v.0;
                    continue;
                }
                Instruction::Copy {
                    src: Operand::Const(_),
                    ..
                } => {
                    return None;
                }
                Instruction::Load { ptr, .. } => {
                    if let Some(stored_val) = alloca_stores.get(&ptr.0) {
                        match stored_val {
                            Operand::Value(v) => {
                                current = v.0;
                                continue;
                            }
                            Operand::Const(_) => return None,
                        }
                    }
                    return None;
                }
                // GEP: accumulate offset (constant or resolvable Value)
                Instruction::GetElementPtr { base, offset, .. } => {
                    let off = match offset {
                        Operand::Const(c) => c.to_i64(),
                        Operand::Value(_) => {
                            trace_operand_to_const(offset, value_defs, alloca_stores, 0)
                        }
                    };
                    if let Some(off) = off {
                        accumulated_offset += off;
                        current = base.0;
                        continue;
                    }
                    return None;
                }
                // Cast preserves pointer identity for address calculations
                Instruction::Cast {
                    src: Operand::Value(v),
                    ..
                } => {
                    current = v.0;
                    continue;
                }
                // BinOp on pointer: handle Add with constant (pointer arithmetic)
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => {
                    // Try: one operand is a traceable pointer, other is a constant offset
                    if let Some(rhs_val) = trace_operand_to_const(rhs, value_defs, alloca_stores, 0)
                    {
                        if let Operand::Value(v) = lhs {
                            accumulated_offset += rhs_val;
                            current = v.0;
                            continue;
                        }
                    }
                    if let Some(lhs_val) = trace_operand_to_const(lhs, value_defs, alloca_stores, 0)
                    {
                        if let Operand::Value(v) = rhs {
                            accumulated_offset += lhs_val;
                            current = v.0;
                            continue;
                        }
                    }
                    return None;
                }
                _ => return None,
            }
        } else {
            return None;
        }
    }
    None
}

/// Recursively trace an operand to find a compile-time constant integer value.
/// Handles Load/Store/Copy/Cast chains as well as BinOp and Cmp with constant operands.
/// `depth` limits recursion for BinOp/Cmp where both sides need tracing.
fn trace_operand_to_const(
    op: &Operand,
    value_defs: &FxHashMap<u32, Instruction>,
    alloca_stores: &FxHashMap<u32, Operand>,
    depth: u32,
) -> Option<i64> {
    if depth > MAX_TRACE_RECURSION_DEPTH {
        return None;
    }
    match op {
        Operand::Const(c) => c.to_i64(),
        Operand::Value(v) => {
            let mut current = v.0;
            for _ in 0..MAX_TRACE_CHAIN_LENGTH {
                if let Some(inst) = value_defs.get(&current) {
                    match inst {
                        Instruction::Copy {
                            src: Operand::Const(c),
                            ..
                        } => {
                            return c.to_i64();
                        }
                        Instruction::Copy {
                            src: Operand::Value(v2),
                            ..
                        } => {
                            current = v2.0;
                            continue;
                        }
                        Instruction::Load { ptr, .. } => {
                            if let Some(stored_val) = alloca_stores.get(&ptr.0) {
                                match stored_val {
                                    Operand::Const(c) => return c.to_i64(),
                                    Operand::Value(v2) => {
                                        current = v2.0;
                                        continue;
                                    }
                                }
                            }
                            return None;
                        }
                        Instruction::Cast { src, .. } => match src {
                            Operand::Const(c) => return c.to_i64(),
                            Operand::Value(v2) => {
                                current = v2.0;
                                continue;
                            }
                        },
                        // Binary operations: try to evaluate both sides
                        Instruction::BinOp {
                            op: bin_op,
                            lhs,
                            rhs,
                            ..
                        } => {
                            let l =
                                trace_operand_to_const(lhs, value_defs, alloca_stores, depth + 1)?;
                            let r =
                                trace_operand_to_const(rhs, value_defs, alloca_stores, depth + 1)?;
                            return bin_op.eval_i64(l, r);
                        }
                        // Comparisons: try to evaluate both sides
                        Instruction::Cmp {
                            op: cmp_op,
                            lhs,
                            rhs,
                            ..
                        } => {
                            let l =
                                trace_operand_to_const(lhs, value_defs, alloca_stores, depth + 1)?;
                            let r =
                                trace_operand_to_const(rhs, value_defs, alloca_stores, depth + 1)?;
                            return Some(if cmp_op.eval_i64(l, r) { 1 } else { 0 });
                        }
                        _ => return None,
                    }
                } else {
                    return None;
                }
            }
            None
        }
    }
}

/// Debug validation: check that every Value used as an operand is defined by some instruction.
fn validate_function_values(func: &IrFunction, last_inlined_callee: &str) {
    use crate::common::fx_hash::FxHashSet;

    // Collect all defined values
    let mut defined: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(v) = inst.dest() {
                defined.insert(v.0);
            }
        }
    }

    // Check all used values
    let mut errors = Vec::with_capacity(16);
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            for v in inst.used_values() {
                if !defined.contains(&v) {
                    errors.push(format!(
                        "  block[{}] (label .L{}) inst[{}]: uses undefined Value({}), inst={:?}",
                        block_idx,
                        block.label.0,
                        inst_idx,
                        v,
                        short_inst_name(inst)
                    ));
                }
            }
        }
        for v in block.terminator.used_values() {
            if !defined.contains(&v) {
                errors.push(format!(
                    "  block[{}] (label .L{}) terminator: uses undefined Value({})",
                    block_idx, block.label.0, v
                ));
            }
        }
    }

    if !errors.is_empty() {
        eprintln!(
            "[INLINE_VALIDATE] ERRORS in '{}' after inlining '{}': {} undefined value uses",
            func.name,
            last_inlined_callee,
            errors.len()
        );
        for e in errors.iter().take(20) {
            eprintln!("{}", e);
        }
        if errors.len() > 20 {
            eprintln!("  ... and {} more", errors.len() - 20);
        }
    }
}

fn short_inst_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::Alloca { .. } => "Alloca",
        Instruction::Store { .. } => "Store",
        Instruction::Load { .. } => "Load",
        Instruction::BinOp { .. } => "BinOp",
        Instruction::UnaryOp { .. } => "UnaryOp",
        Instruction::Cmp { .. } => "Cmp",
        Instruction::Call { .. } => "Call",
        Instruction::CallIndirect { .. } => "CallIndirect",
        Instruction::GetElementPtr { .. } => "GEP",
        Instruction::Cast { .. } => "Cast",
        Instruction::Copy { .. } => "Copy",
        Instruction::GlobalAddr { .. } => "GlobalAddr",
        Instruction::Memcpy { .. } => "Memcpy",
        Instruction::Phi { .. } => "Phi",
        Instruction::Select { .. } => "Select",
        _ => "Other",
    }
}

/// ABI metadata for a call site's argument list (S17 va_arg_pack forwarding).
/// Parallel arrays lifted from the caller's `CallInfo` so that forwarded
/// variadic arguments can be spliced into an inlined wrapper body's call with
/// a correct ABI description (struct-by-value sizes/classes matter: glibc's
/// _FORTIFY_SOURCE wrappers forward 16-byte structs through va_arg_pack).
#[derive(Clone, Default)]
struct CallArgMeta {
    arg_types: Vec<IrType>,
    struct_arg_sizes: Vec<Option<usize>>,
    struct_arg_aligns: Vec<Option<usize>>,
    struct_arg_classes: Vec<Vec<EightbyteClass>>,
    struct_arg_riscv_float_classes: Vec<Option<RiscvFloatClass>>,
    struct_arg_is_f128_sse: Vec<bool>,
}

impl CallArgMeta {
    fn from_call_info(info: &CallInfo) -> Self {
        CallArgMeta {
            arg_types: info.arg_types.clone(),
            struct_arg_sizes: info.struct_arg_sizes.clone(),
            struct_arg_aligns: info.struct_arg_aligns.clone(),
            struct_arg_classes: info.struct_arg_classes.clone(),
            struct_arg_riscv_float_classes: info.struct_arg_riscv_float_classes.clone(),
            struct_arg_is_f128_sse: info.struct_arg_is_f128_sse.clone(),
        }
    }

    /// Extract the metadata slice for arguments `[from..to]`.
    fn slice(&self, from: usize, to: usize) -> CallArgMeta {
        let take = |v: &[IrType]| -> Vec<IrType> { v[from.min(v.len())..to.min(v.len())].to_vec() };
        CallArgMeta {
            arg_types: take(&self.arg_types),
            struct_arg_sizes: self.struct_arg_sizes
                [from.min(self.struct_arg_sizes.len())..to.min(self.struct_arg_sizes.len())]
                .to_vec(),
            struct_arg_aligns: self.struct_arg_aligns
                [from.min(self.struct_arg_aligns.len())..to.min(self.struct_arg_aligns.len())]
                .to_vec(),
            struct_arg_classes: self.struct_arg_classes
                [from.min(self.struct_arg_classes.len())..to.min(self.struct_arg_classes.len())]
                .to_vec(),
            struct_arg_riscv_float_classes: self.struct_arg_riscv_float_classes[from
                .min(self.struct_arg_riscv_float_classes.len())
                ..to.min(self.struct_arg_riscv_float_classes.len())]
                .to_vec(),
            struct_arg_is_f128_sse: self.struct_arg_is_f128_sse[from
                .min(self.struct_arg_is_f128_sse.len())
                ..to.min(self.struct_arg_is_f128_sse.len())]
                .to_vec(),
        }
    }
}

/// `__builtin_va_arg_pack()` forwarding plan for an always_inline variadic
/// wrapper (S17).  The lowering emits zero-arg sentinel calls
/// (`__lccc_va_arg_pack` / `__lccc_va_arg_pack_len`); this records their
/// callee-space destination values so `inline_call_site` can delete the
/// sentinels and splice the call site's arguments beyond the wrapper's named
/// parameters into the consuming call.  `vp_value` uses are validated to be
/// direct arguments of Call instructions only; `len_values` become I32
/// constants (rewritten as `Copy { src: Const }`, so any use shape works).
#[derive(Clone, Default)]
struct VaArgPackPlan {
    /// Destination values of ALL `__lccc_va_arg_pack` sentinel calls.  The
    /// C source may use `__builtin_va_arg_pack()` several times (one per
    /// return path — gcc.c-torture va-arg-pack-1.c does exactly that); each
    /// use expands to the same forwarded argument list, and because the
    /// forwarded operands are already-evaluated SSA values, splicing the
    /// same list at every consuming call is sound and evaluates each
    /// caller argument exactly once.
    vp_values: Vec<Value>,
    len_values: Vec<Value>,
}

impl VaArgPackPlan {
    fn is_empty(&self) -> bool {
        self.vp_values.is_empty() && self.len_values.is_empty()
    }
}

/// Analyze `func` for va_arg_pack forwarding.  Returns None when the body
/// uses the sentinels in a way the splice cannot express (multiple vp
/// sentinel calls, or the vp value consumed outside a call argument list).
fn analyze_va_arg_pack(func: &IrFunction) -> Option<VaArgPackPlan> {
    const VP: &str = "__lccc_va_arg_pack";
    const VPLEN: &str = "__lccc_va_arg_pack_len";
    let mut plan = VaArgPackPlan::default();
    let mut vp_invalid = false;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Call { func: f, info } = inst {
                if f == VP {
                    if let Some(d) = info.dest {
                        plan.vp_values.push(d);
                    }
                } else if f == VPLEN {
                    plan.len_values.push(info.dest?);
                }
            }
        }
    }
    if plan.is_empty() {
        return None;
    }
    for v in plan.vp_values.clone() {
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Call { func: f, info } => {
                        if f == VP || f == VPLEN {
                            continue;
                        }
                        // The vp value may appear ONLY among the call's
                        // forwarded arguments (where splicing is defined).
                        // Direct calls have a String target; nothing else in
                        // a direct Call can reference the value.
                    }
                    Instruction::CallIndirect { func_ptr, .. } => {
                        if func_ptr == &Operand::Value(v) {
                            return None;
                        }
                    }
                    other => {
                        let mut bad = false;
                        other.for_each_used_value(|u| {
                            if u == v.0 {
                                bad = true;
                            }
                        });
                        if bad {
                            return None;
                        }
                    }
                }
            }
            block.terminator.for_each_used_value(|u| {
                if u == v.0 {
                    vp_invalid = true;
                }
            });
        }
        if vp_invalid {
            return None;
        }
    }
    Some(plan)
}

#[derive(Clone)]
struct CalleeData {
    blocks: Vec<BasicBlock>,
    /// For each param, Some(size) if it's a struct-by-value parameter, None otherwise.
    param_struct_sizes: Vec<Option<usize>>,
    return_type: IrType,
    num_params: usize,
    next_value_id: u32,
    /// Maximum BlockId used in the callee
    max_block_id: u32,
    /// Whether this callee was marked __attribute__((always_inline))
    is_always_inline: bool,
    /// True if this callee exceeds normal inline limits but is within the
    /// relaxed limits used for callers with custom section attributes.
    /// Such callees should only be inlined when the caller has a section attribute,
    /// to avoid cross-section calls that break early boot / noinstr code.
    exceeds_normal_limits: bool,
    /// Whether this callee is a `static inline` function. GCC always inlines
    /// `static inline` functions regardless of caller size. We should do the
    /// same to match GCC behavior and enable critical optimizations (e.g.,
    /// constant propagation of shift amounts in ror32 used by blake2s).
    is_static_inline: bool,
    /// GNU89 `extern inline __attribute__((gnu_inline))` definition: the body
    /// exists ONLY for inlining — no out-of-line copy is emitted in this TU.
    /// A call left behind resolves to an external symbol that may not exist
    /// (glibc rtld links -nostdlib: `__bsearch` in intel_check_word was a
    /// hard undefined-symbol error). Eligibility uses dedicated relaxed caps.
    is_gnu_inline_def: bool,
    /// An ordinary `static` definition, rather than a static-inline or an
    /// attribute-constrained inline definition. It has a valid outlined body,
    /// so profitability policy may deliberately retain a call to it.
    is_plain_static: bool,
    /// Module-wide direct calls (including a redirected asm label) observed
    /// before inlining.  Multi-site wrappers duplicate their descendants.
    direct_call_count: usize,
    /// Whether recursively expanding ordinary eligible callees reaches a loop.
    /// A tiny wrapper can otherwise conceal a large loop nest from the raw
    /// tiny/small threshold used by the fixed-point inliner.
    has_inlineable_loop_descendant: bool,
    /// A prior inliner invocation has already expanded a descendant into this
    /// body. This persists across pipeline invocations, unlike a fresh raw
    /// call-graph snapshot, and identifies wrappers that have acquired a loop
    /// body after their initial tiny/small eligibility decision.
    has_inlined_calls: bool,
    /// Whether this callee is a `static` (non-`inline`) function with exactly
    /// one call site in the whole module. Such callees are dead after inlining
    /// (net code shrink), so they are exempt from the caller-size caps and the
    /// per-caller inlining budget, matching GCC's -O2 behavior.
    is_single_call_site_static: bool,
    /// Whether this callee contains any back-edges (loops).
    /// Functions without loops can use a higher block limit for inlining.
    has_loops: bool,
    /// Direct self-recursion. Normal inlining rejects these callees: after one
    /// clone the recursive call remains in the caller and a fixed-point inliner
    /// would keep cloning the body until the caller size cap. Recursive
    /// specialization is a separate bounded transform.
    is_recursive: bool,
    /// Whether this callee contains SIMD vector intrinsics (SSE/AVX/AVX2).
    /// Functions dominated by vector intrinsics are allowed a much larger
    /// static-inline budget: their standalone codegen is memory-bound, and
    /// inlining lets the caller fuse the intrinsic chain.
    has_vector_intrinsics: bool,
    /// Conservative instruction cost after descendants that the first-pass
    /// fixed-point policy will necessarily inline have expanded.  -Os uses
    /// this rather than the stale raw snapshot for its nested-loop limit.
    size_inline_cost: usize,
    /// va_arg_pack forwarding plan (S17): present only for variadic
    /// always_inline wrappers whose sentinel uses validate.  Empty for every
    /// other callee.
    va_arg_pack: Option<VaArgPackPlan>,
}

/// A call site that is eligible for inlining.
#[derive(Clone)]
struct InlineCallSite {
    /// Index of the block containing the call
    block_idx: usize,
    /// Index of the instruction within the block
    inst_idx: usize,
    /// Name of the callee function
    callee_name: String,
    /// The destination value of the call (None for void)
    dest: Option<Value>,
    /// Arguments passed to the call
    args: Vec<Operand>,
    /// Profile-guided force-inline. Set by the PGO filter when the call
    /// site is genuinely HOT (in a hot loop / high frequency) and should be
    /// inlined even if the base inliner's caller-size or budget limits would
    /// normally skip it — the PGO advantage LLVM/ICC have over a plain build.
    /// Still bounded by a dedicated budget to prevent unbounded bloat.
    pgo_force: bool,
    /// ABI metadata of the call's argument list (S17): required to splice
    /// forwarded variadic arguments (va_arg_pack) with correct types and
    /// struct-by-value classification.
    arg_meta: CallArgMeta,
}

/// Check if a GlobalInit contains references to local labels (`.LBBxx`).
/// These are produced by `&&label` (label-as-value) in static local initializers.
fn global_init_contains_local_label(init: &GlobalInit) -> bool {
    match init {
        GlobalInit::GlobalAddr(label) => label.starts_with(".LBB"),
        GlobalInit::GlobalLabelDiff(lab1, lab2, _) => {
            lab1.starts_with(".LBB") || lab2.starts_with(".LBB")
        }
        GlobalInit::Compound(inits) => inits.iter().any(global_init_contains_local_label),
        _ => false,
    }
}

/// Check if a function has static local variables whose initializers reference
/// local labels (from `&&label`). Such functions cannot be safely inlined because
/// the label references in static data are stored as strings and are NOT remapped
/// when the function body's block IDs are remapped during inlining.
fn func_has_static_locals_with_label_refs(module: &IrModule, func_name: &str) -> bool {
    let prefix = format!("{}.", func_name);
    for global in &module.globals {
        if global.name.starts_with(&prefix) && global_init_contains_local_label(&global.init) {
            return true;
        }
    }
    false
}

/// Build a map of function name -> callee data for functions eligible for inlining.
fn fits_normal_inline_limits(
    inst_count: usize,
    block_count: usize,
    is_static: bool,
    is_inline: bool,
    has_vector_intrinsics: bool,
    block_limit: usize,
) -> bool {
    let inst_ok = inst_count <= MAX_INLINE_INSTRUCTIONS
        || (is_static && !is_inline && inst_count <= MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS)
        || (is_static
            && is_inline
            && has_vector_intrinsics
            && inst_count <= MAX_VECTOR_STATIC_INLINE_INSTRUCTIONS);
    let block_ok = if is_static && is_inline && has_vector_intrinsics {
        block_count <= MAX_VECTOR_STATIC_INLINE_BLOCKS
    } else {
        block_count <= block_limit
    };
    inst_ok && block_ok
}

fn fits_static_loop_inline_limits(
    inst_count: usize,
    block_count: usize,
    direct_call_count: usize,
) -> bool {
    let small = inst_count <= MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS
        && block_count <= MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS;
    let bounded_clone = direct_call_count <= MAX_STATIC_LOOP_INLINE_CALLS
        && inst_count <= MAX_STATIC_LOOP_INLINE_INSTRUCTIONS
        && block_count <= MAX_STATIC_LOOP_INLINE_BLOCKS;
    small || bounded_clone
}

fn direct_call_counts(module: &IrModule) -> FxHashMap<String, usize> {
    let mut counts = FxHashMap::default();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let Instruction::Call { func, .. } = instruction {
                    *counts.entry(func.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    counts
}

/// Names of functions referenced as VALUES anywhere in the module outside of
/// direct `Call` instructions: via `&func` (`GlobalAddr`), in global
/// initializer tables (`GlobalInit::GlobalAddr...`), as an inline-asm "i"
/// constraint symbol, inside an inline-asm template string, as an alias
/// target, or as a constructor/destructor.
///
/// This is the soundness precondition for the single-call-site-static
/// exemption: a function referenced as a value SURVIVES the inlining of its
/// one direct call (the reference still needs the outlined body), so the
/// "callee is dead after inlining, net code shrink" argument does not apply,
/// and neither the budget exemption nor the uncapped admission may fire.
/// Inline-asm input operands that name symbols directly are covered through
/// `input_symbols`; the template text itself can still name a symbol
/// (`asm("call foo")`), hence the conservative substring scan over the
/// (typically very few) templates.
fn collect_value_referenced_functions(module: &IrModule) -> (FxHashSet<String>, Vec<String>) {
    let mut names: FxHashSet<String> = FxHashSet::default();
    let mut asm_templates: Vec<String> = Vec::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                match instruction {
                    Instruction::GlobalAddr { name, .. } => {
                        names.insert(name.clone());
                    }
                    Instruction::InlineAsm {
                        template,
                        input_symbols,
                        ..
                    } => {
                        asm_templates.push(template.clone());
                        for sym in input_symbols {
                            if let Some(s) = sym {
                                names.insert(s.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for global in &module.globals {
        global.init.for_each_ref(&mut |r| {
            names.insert(r.to_string());
        });
    }
    for (_, target, _) in &module.aliases {
        names.insert(target.clone());
    }
    for ctor in &module.constructors {
        names.insert(ctor.clone());
    }
    for dtor in &module.destructors {
        names.insert(dtor.clone());
    }
    (names, asm_templates)
}

/// Whether `name` is referenced as a value (see
/// `collect_value_referenced_functions` for what counts).
fn function_referenced_as_value(
    name: &str,
    referenced: &FxHashSet<String>,
    asm_templates: &[String],
) -> bool {
    if referenced.contains(name) {
        return true;
    }
    asm_templates.iter().any(|t| t.contains(name))
}

fn build_callee_map(module: &IrModule) -> FxHashMap<String, CalleeData> {
    let call_counts = direct_call_counts(module);
    let mut map = FxHashMap::default();

    // One linear scan: which functions are referenced as values anywhere?
    // Needed before the per-function loop below (see the soundness note on
    // `is_single_call_site_static`).
    let (value_referenced, asm_templates) = collect_value_referenced_functions(module);

    // Count direct call sites per callee across the whole module. A static
    // function with exactly one call site is dead after inlining (net code
    // shrink), so it gets more generous size limits below.
    let mut call_site_counts: FxHashMap<String, usize> = FxHashMap::default();
    for f in &module.functions {
        for block in &f.blocks {
            for inst in &block.instructions {
                if let Instruction::Call {
                    func: callee_name, ..
                } = inst
                {
                    *call_site_counts.entry(callee_name.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    let debug_callee = std::env::var("CCC_INLINE_DEBUG").is_ok();
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        // __attribute__((noinline)) takes precedence: never inline these functions
        if func.is_noinline {
            continue;
        }
        // __weak definitions are NOT necessarily the linked definition: a
        // strong override in another TU replaces the symbol at link time,
        // and the linker resolves every UN-inlined call to the strong body.
        // Cloning the weak body into a call site bypasses that resolution
        // (kernel 6.18 sparse-vmemmap.c: the __weak empty vmemmap_set_pmd
        // was inlined away while arch/x86/mm/init_64.c's strong override
        // installs the PMD — vmemmap never got mapped and
        // __init_single_page page-faulted on pfn 1). Weak callees are
        // therefore never inlinable, no matter how tiny their body.
        if func.is_weak {
            continue;
        }

        // va_arg_pack forwarding plan (S17) — computed before the variadic
        // gate below, consumed by the CalleeData construction at the loop tail.
        let mut va_arg_pack_plan: Option<VaArgPackPlan> = None;

        // Determine if this is an always_inline function
        let is_always_inline = func.is_always_inline;

        // For always_inline: we inline regardless of whether the function is static,
        // because GCC/Clang semantics dictate that __attribute__((always_inline))
        // means the function must always be inlined at call sites.
        // For normal inlining: only inline static inline functions (internal linkage),
        // OR static functions that are trivially empty (void return, no instructions).
        // Empty static stubs must be inlined so that references to their arguments
        // (which may be undefined symbols) are eliminated by DCE. This matches GCC
        // behavior where empty stub functions like `static void __apply_fineibt(...){}`
        // are inlined away, removing references to symbols like __cfi_sites.
        // A function is "trivially empty" if it is a static void function
        // with a single block whose only instructions are parameter allocas
        // (which are always generated by the lowering even for empty bodies)
        // and terminates with Return(None).
        let is_trivially_empty = func.is_static
            && func.return_type == IrType::Void
            && func.blocks.len() == 1
            && matches!(func.blocks[0].terminator, Terminator::Return(None))
            && func.blocks[0]
                .instructions
                .iter()
                .all(|inst| matches!(inst, Instruction::Alloca { .. }));
        // Check if this is a small static (non-inline) function eligible for inlining.
        // GCC at -O2 inlines small static functions even without the `inline` keyword.
        // This prevents linker errors from undefined references in dead code paths.
        let inst_count_for_static: usize = func.blocks.iter().map(|b| b.instructions.len()).sum();

        // Detect back-edges (loops) early, used for block limit decision.
        let has_loops = {
            let label_to_order: FxInlineMap<BlockId, usize> = func
                .blocks
                .iter()
                .enumerate()
                .map(|(i, b)| (b.label, i))
                .collect();
            func.blocks.iter().enumerate().any(|(i, block)| {
                let succs: Vec<BlockId> = match &block.terminator {
                    Terminator::Branch(t) => vec![*t],
                    Terminator::CondBranch {
                        true_label,
                        false_label,
                        ..
                    } => vec![*true_label, *false_label],
                    Terminator::Switch { default, cases, .. } => {
                        let mut s = vec![*default];
                        s.extend(cases.iter().map(|(_, l)| *l));
                        s
                    }
                    _ => vec![],
                };
                succs
                    .iter()
                    .any(|succ| label_to_order.get(succ).map_or(false, |&j| j <= i))
            })
        };
        let is_recursive = func.blocks.iter().any(|block| {
            block.instructions.iter().any(|inst| {
                matches!(
                    inst,
                    Instruction::Call { func: callee_name, .. }
                        if callee_name == &func.name
                )
            })
        });
        let direct_call_count = call_counts.get(&func.name).copied().unwrap_or(0);
        let is_small_static = func.is_static
            && !func.is_inline
            && inst_count_for_static <= MAX_STATIC_NONINLINE_INSTRUCTIONS
            && func.blocks.len() <= MAX_STATIC_NONINLINE_BLOCKS;
        // Also consider medium-sized static non-inline functions for inlining.
        // GCC at -O2/-Os inlines static functions up to a generous limit even without
        // the `inline` keyword. This is critical for avoiding section mismatch errors
        // in the kernel: e.g., ssb_select_mitigation (~6 blocks, ~21 IR instructions)
        // is called from cpu_select_mitigations (.init.text) and calls
        // __ssb_select_mitigation (.init.text). Without inlining the wrapper,
        // modpost flags a .text -> .init.text section mismatch.
        // These are treated as normal inline candidates (not exceeds_normal_limits)
        // since they fit within MAX_INLINE_INSTRUCTIONS/MAX_INLINE_BLOCKS.
        let medium_block_limit = if func.is_static && !func.is_inline {
            MAX_MEDIUM_STATIC_INLINE_BLOCKS
        } else if has_loops {
            MAX_INLINE_BLOCKS
        } else {
            MAX_INLINE_BLOCKS_NO_LOOPS
        };
        // Medium static callees up to MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS
        // instructions are admitted, including small loop CFGs. This exposes
        // aggregate-return helpers such as make_group to scalar cleanup. The
        // per-caller budget and caller-size cap remain the code-growth bounds.
        let is_medium_static = func.is_static
            && !func.is_inline
            && !is_small_static
            && ((!has_loops
                && inst_count_for_static <= MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS
                && func.blocks.len() <= medium_block_limit)
                || (has_loops
                    && fits_static_loop_inline_limits(
                        inst_count_for_static,
                        func.blocks.len(),
                        direct_call_count,
                    )));
        // A static (non-inline) function with exactly one call site in the
        // module is dead after inlining (net code shrink), so GCC -O2 inlines
        // it even when it exceeds the multi-call-site static limits above.
        // Admit these with the more generous single-call-site limits.
        // Soundness: the callee only dies if its address is never taken as a
        // value anywhere (function-pointer table, alias, constructor, asm
        // template or "i"-constraint symbol); otherwise the outlined body
        // survives and the exemption must not apply.
        // glibc hidden_proto: call sites are redirected to the
        // __asm__("__GI_...") label while the body keeps its C name, so the
        // call accounting and the value-reference check must consider both.
        let asm_alias = module
            .asm_labels
            .get(&func.name)
            .filter(|a| a.as_str() != func.name.as_str());
        let total_calls = call_site_counts.get(&func.name).copied().unwrap_or(0)
            + asm_alias
                .map(|a| call_site_counts.get(a.as_str()).copied().unwrap_or(0))
                .unwrap_or(0);
        let has_single_call_site = total_calls == 1;
        let survives_via_reference = has_single_call_site
            && (function_referenced_as_value(&func.name, &value_referenced, &asm_templates)
                || asm_alias.is_some_and(|a| {
                    function_referenced_as_value(a, &value_referenced, &asm_templates)
                }));
        let fits_single_call_site_static = func.is_static
            && !func.is_inline
            && !is_small_static
            && !is_medium_static
            && has_single_call_site
            && !survives_via_reference
            && (inst_count_for_static <= MAX_SINGLE_CALL_SITE_STATIC_INSTRUCTIONS
                && func.blocks.len() <= MAX_SINGLE_CALL_SITE_STATIC_BLOCKS);
        if !is_always_inline
            && !is_trivially_empty
            && !is_small_static
            && !is_medium_static
            && !fits_single_call_site_static
            // An external-linkage `inline` definition is still an inlining
            // candidate; linkage controls out-of-line emission, not whether
            // callers in this TU may use the body. Excluding it made
            // __builtin_constant_p(parameter) specialization impossible and
            // contradicted both C99 and GNU89 inline behavior.
            && !func.is_inline
            && !func.is_gnu_inline_def
        {
            if debug_callee {
                eprintln!(
                    "[INLINE_DEBUG] {} skipped: is_static={}, is_inline={}, blocks={}, inst_count={}, has_loops={}, direct_calls={}, medium_block_limit={}, single_site={}, survives_via_reference={}, is_declaration={}",
                    func.name,
                    func.is_static,
                    func.is_inline,
                    func.blocks.len(),
                    inst_count_for_static,
                    has_loops,
                    direct_call_count,
                    medium_block_limit,
                    has_single_call_site,
                    survives_via_reference,
                    func.is_declaration,
                );
            }
            continue;
        }
        if debug_callee {
            let ic: usize = func.blocks.iter().map(|b| b.instructions.len()).sum();
            eprintln!(
                "[INLINE_DEBUG] {} candidate: blocks={}, inst_count={}, direct_calls={}, is_variadic={}, params={}",
                func.name,
                func.blocks.len(),
                ic,
                direct_call_count,
                func.is_variadic,
                func.params.len()
            );
        }
        // Don't inline variadic functions (complex ABI) — EXCEPT the
        // always_inline wrapper shape whose only variadic forwarding is a
        // validated __builtin_va_arg_pack()/__builtin_va_arg_pack_len()
        // sentinel pair (glibc _FORTIFY_SOURCE wrappers,
        // gcc.c-torture/execute/va-arg-pack-1.c).  GCC always inlines those;
        // with gnu_inline semantics an un-inlined wrapper has no out-of-line
        // body, so refusing to inline them is a hard link error.
        if func.is_variadic {
            let plan = if func.is_always_inline {
                analyze_va_arg_pack(func)
            } else {
                None
            };
            match plan {
                Some(p) => {
                    va_arg_pack_plan = Some(p);
                }
                None => {
                    continue;
                }
            }
        }

        // Check size limits.
        // For always_inline: use generous limits.
        // For static inline: use normal limits, but also admit functions that exceed
        // normal limits but fit within always_inline limits. These "exceeds_normal_limits"
        // callees will only be inlined when the caller has a custom section attribute
        // (e.g., .head.text, .noinstr.text), where cross-section calls are dangerous.
        let inst_count: usize = func.blocks.iter().map(|b| b.instructions.len()).sum();
        // Non-loop callees get a higher block limit since their control flow
        // (if/else chains, switch, early returns) doesn't create nested loops
        // when inlined into a loop caller.
        let effective_block_limit = if func.is_static && !func.is_inline {
            MAX_MEDIUM_STATIC_INLINE_BLOCKS
        } else if has_loops {
            MAX_INLINE_BLOCKS
        } else {
            MAX_INLINE_BLOCKS_NO_LOOPS
        };
        // ms178: static (non-inline) callees get a higher instruction limit
        // (MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS). Hot leaf helpers like expat's
        // sip_round (~85 instr, one loop) are otherwise marked
        // exceeds_normal_limits and excluded from ordinary callers, leaving
        // their state in memory. The per-caller budget bounds the growth.
        let fits_normal = fits_normal_inline_limits(
            inst_count,
            func.blocks.len(),
            func.is_static,
            func.is_inline,
            func_has_vector_intrinsics(func),
            effective_block_limit,
        ) || (func.is_static
            && !func.is_inline
            && has_loops
            && fits_static_loop_inline_limits(inst_count, func.blocks.len(), direct_call_count))
            // GNU89 `extern inline __attribute__((gnu_inline))` bodies exist
            // ONLY for inlining (no out-of-line copy is emitted in this TU);
            // a rejected site leaves a call to an external symbol that may
            // not exist (glibc rtld __bsearch). Dedicated caps: big enough
            // for glibc's header bodies, still bounded.
            || (func.is_gnu_inline_def && inst_count <= 128 && func.blocks.len() <= 16);
        let fits_relaxed = inst_count <= MAX_ALWAYS_INLINE_INSTRUCTIONS
            && func.blocks.len() <= MAX_ALWAYS_INLINE_BLOCKS;
        // Single-call-site statics were already size-checked above against
        // their own limits; treat them as fitting normal limits so they are
        // inlinable into ordinary callers (not just section-attributed ones).
        let exceeds_normal = !is_always_inline && !fits_normal && !fits_single_call_site_static;
        if is_always_inline {
            if !fits_relaxed {
                continue;
            }
        } else {
            // For static inline: admit if within normal limits OR within relaxed limits
            // (the latter only used for section-attributed callers).
            if !fits_normal && !fits_single_call_site_static && !fits_relaxed {
                continue;
            }
        }

        // Skip functions containing constructs that are hard to inline correctly.
        // Inline asm is allowed: the inliner handles it correctly and the
        // resolve_inline_asm_symbols post-pass resolves operand symbols.
        // This is important because many kernel static inline functions use
        // inline asm (cr reads/writes, atomic ops, barriers, RIP_REL_REF, etc.)
        // and must be inlinable to avoid cross-section call issues.
        let mut has_problematic = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    // Inline asm is allowed for all inlined functions
                    Instruction::InlineAsm { .. } => {}
                    Instruction::VaStart { .. }
                    | Instruction::VaEnd { .. }
                    | Instruction::VaArg { .. }
                    | Instruction::VaArgStruct { .. }
                    | Instruction::VaCopy { .. }
                    | Instruction::DynAlloca { .. }
                    | Instruction::StackSave { .. }
                    | Instruction::StackRestore { .. } => {
                        has_problematic = true;
                        break;
                    }
                    _ => {}
                }
            }
            if has_problematic {
                break;
            }
            if matches!(block.terminator, Terminator::IndirectBranch { .. }) {
                has_problematic = true;
                break;
            }
        }
        if has_problematic {
            continue;
        }

        // Don't inline functions whose static local variables contain label
        // address references (&&label). The label references in static data are
        // stored as assembly label strings (e.g., ".L3") and are NOT remapped
        // when block IDs are remapped during inlining. This causes dangling
        // references to non-existent labels, resulting in linker errors.
        if func_has_static_locals_with_label_refs(module, &func.name) {
            continue;
        }

        // Clone the function's blocks for use during inlining
        let max_block_id = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);

        let param_struct_sizes: Vec<Option<usize>> =
            func.params.iter().map(|p| p.struct_size).collect();

        map.insert(
            func.name.clone(),
            CalleeData {
                blocks: func.blocks.clone(),
                param_struct_sizes,
                return_type: func.return_type,
                num_params: func.params.len(),
                // Post-structural inlining can clone values into a function
                // whose cached next_value_id predates the clone. Preserve the
                // stronger of the cached cursor and the real IR maximum so a
                // later inline cannot collide with an existing definition.
                next_value_id: std::cmp::max(
                    func.next_value_id,
                    func.max_value_id().saturating_add(1),
                ),
                max_block_id,
                is_always_inline,
                exceeds_normal_limits: exceeds_normal,
                is_static_inline: func.is_static && func.is_inline,
                is_gnu_inline_def: func.is_gnu_inline_def,
                is_plain_static: func.is_static
                    && !func.is_inline
                    && !is_always_inline
                    && !func.is_gnu_inline_def,
                direct_call_count: total_calls,
                has_inlineable_loop_descendant: false,
                has_inlined_calls: func.has_inlined_calls,
                // Any static (non-inline) callee with a single call site is
                // dead after inlining, so it is exempt from the soft caller-
                // size cap and the per-caller budget in select_inline_site
                // (regardless of which size bucket above admitted it).
                is_single_call_site_static: func.is_static
                    && !func.is_inline
                    && has_single_call_site
                    && !survives_via_reference,
                has_loops,
                is_recursive,
                has_vector_intrinsics: func_has_vector_intrinsics(func),
                // Filled from the complete map below, after every eligible
                // descendant is known.
                size_inline_cost: inst_count,
                va_arg_pack: va_arg_pack_plan.take(),
            },
        );
        // Register the same body under its __asm__ label so call sites that
        // carry the redirected name find the inline definition. Without this,
        // glibc's hidden_proto pattern (body `__argz_next`, calls
        // `__GI___argz_next`) left the gnu89 extern-inline body uninlined and
        // the libc.so link failed with undefined `__GI___argz_next` /
        // `__option_is_end` / `__option_is_short`.
        if let Some(asm_name) = module.asm_labels.get(&func.name) {
            if asm_name.as_str() != func.name.as_str() && !map.contains_key(asm_name.as_str()) {
                let aliased = map
                    .get(&func.name)
                    .cloned()
                    .expect("entry inserted just above");
                map.insert(asm_name.clone(), aliased);
            }
        }
    }

    // Callee snapshots are captured before any function in this invocation is
    // processed.  Estimate the body that fixed-point inlining will actually
    // clone by recursively charging descendants selected by the unconditional
    // first pass (tiny/small, static-inline, and always_inline).  This closes a
    // size-policy hole without changing the cloned IR or the normal -O2/-O3
    // profitability model.
    let names: Vec<String> = map.keys().cloned().collect();
    let costs: Vec<(String, usize)> = names
        .into_iter()
        .map(|name| {
            let mut visiting = FxHashSet::default();
            let cost = estimate_size_inline_cost(&name, &map, &mut visiting, 0);
            (name, cost)
        })
        .collect();
    for (name, cost) in costs {
        if let Some(data) = map.get_mut(&name) {
            data.size_inline_cost = cost;
        }
    }
    let names: Vec<String> = map.keys().cloned().collect();
    let loop_descendants: Vec<(String, bool)> = names
        .into_iter()
        .map(|name| {
            let mut visiting = FxHashSet::default();
            let has_loop = inlineable_loop_descendant(&name, &map, &mut visiting, 0);
            (name, has_loop)
        })
        .collect();
    for (name, has_loop) in loop_descendants {
        if let Some(data) = map.get_mut(&name) {
            data.has_inlineable_loop_descendant = has_loop;
        }
    }

    map
}

/// A recursively reachable callee which normal -O2/-O3 policy may expand and
/// which contains a loop.  This is deliberately a one-bit diagnostic, not a
/// speculative profitability estimate: its sole job is to prevent a tiny
/// *multi-site* wrapper from bypassing every existing loop-nest guard.
fn inlineable_loop_descendant(
    name: &str,
    map: &FxHashMap<String, CalleeData>,
    visiting: &mut FxHashSet<String>,
    depth: usize,
) -> bool {
    let Some(data) = map.get(name) else {
        return false;
    };
    if depth >= 16 || !visiting.insert(name.to_string()) {
        return false;
    }
    let mut found = false;
    for block in &data.blocks {
        for inst in &block.instructions {
            let Instruction::Call {
                func: child_name, ..
            } = inst
            else {
                continue;
            };
            let Some(child) = map.get(child_name) else {
                continue;
            };
            // Recursive normal callees are rejected by select_inline_site,
            // and a relaxed-only body cannot expand in the ordinary path.
            if (child.is_recursive && !child.is_always_inline)
                || (child.exceeds_normal_limits && !child.is_always_inline)
            {
                continue;
            }
            if child.has_loops || inlineable_loop_descendant(child_name, map, visiting, depth + 1) {
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    visiting.remove(name);
    found
}

/// Preserve an ordinary tiny/small static wrapper when inlining it would
/// replicate a reachable loop body at multiple sites.  A custom-section caller
/// may require inlining for section correctness and PGO force-inlining has
/// better hotness information, so neither is overridden here.
fn multisite_loop_wrapper_should_stay_outlined(
    data: &CalleeData,
    raw_inst_count: usize,
    caller_has_section: bool,
    pgo_force: bool,
    size_optimized: bool,
) -> bool {
    let fresh_small_wrapper = data.has_inlineable_loop_descendant
        && raw_inst_count <= MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS
        && data.blocks.len() <= MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS;
    // A later pipeline invocation sees the expanded body, not the original
    // wrapper.  Preserve the decision when the function itself now contains
    // the acquired loop; otherwise it would escape the first-pass guard just
    // because inlining made it larger than the small-wrapper limits.
    let previously_expanded_wrapper = data.has_loops && data.has_inlined_calls;
    !size_optimized
        && data.is_plain_static
        && !data.is_single_call_site_static
        && data.direct_call_count > 1
        // A fresh small loop helper can still be a worthwhile multi-site
        // inline. This guard is narrowly for a wrapper that either hides a
        // loop descendant now, or acquired one in an earlier pipeline pass.
        && (fresh_small_wrapper || previously_expanded_wrapper)
        && !caller_has_section
        && !pgo_force
}

/// An ordinary static loop body called at several syntactic sites from an
/// enclosing loop.  LCCC's current allocator often loses more to the merged
/// live sets than it saves in call overhead: the workload-derived `lookup`
/// loop became 16.8% faster when it remained outlined.  This deliberately
/// does not apply at -Os (whose size policy has separately measured bounded
/// hash-loop exceptions), to explicit inline forms, custom-section callers,
/// profile-forced sites, or a single owner.
fn nested_loop_multisite_static_should_stay_outlined(
    data: &CalleeData,
    raw_inst_count: usize,
    call_is_in_loop: bool,
    caller_has_section: bool,
    pgo_force: bool,
    size_optimized: bool,
) -> bool {
    !size_optimized
        && data.is_plain_static
        && !data.is_single_call_site_static
        && data.direct_call_count > 1
        && data.has_loops
        && raw_inst_count > MAX_TINY_INLINE_INSTRUCTIONS
        && call_is_in_loop
        && !caller_has_section
        && !pgo_force
}

/// Cap repeated cloning of an ordinary small loop helper even when its calls
/// are in distinct branches rather than syntactically inside a caller loop.
/// At five 27-instruction clones, the glibc memcmp decision grew its caller
/// from 93 to 183 instructions, introduced 18 stack references, and was
/// 7.1% slower in an alternating runtime screen.
fn repeated_small_loop_clone_should_stay_outlined(
    data: &CalleeData,
    raw_inst_count: usize,
    caller_has_section: bool,
    pgo_force: bool,
    size_optimized: bool,
) -> bool {
    !size_optimized
        && data.is_plain_static
        && data.has_loops
        && data.direct_call_count > MAX_SMALL_STATIC_LOOP_INLINE_CLONES
        && raw_inst_count > MAX_TINY_INLINE_INSTRUCTIONS
        && raw_inst_count <= MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS
        && data.blocks.len() <= MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS
        && !caller_has_section
        && !pgo_force
}

/// Return whether a descendant is selected by the inliner's unconditional
/// first pass, independent of -Os normal-callee profitability gates.
fn is_mandatory_first_pass_callee(data: &CalleeData) -> bool {
    let inst_count: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
    let is_tiny = inst_count <= MAX_TINY_INLINE_INSTRUCTIONS && data.blocks.len() <= 1;
    let is_small =
        inst_count <= MAX_SMALL_INLINE_INSTRUCTIONS && data.blocks.len() <= MAX_SMALL_INLINE_BLOCKS;
    let static_inline_block_limit = if data.has_loops {
        MAX_INLINE_BLOCKS
    } else {
        MAX_INLINE_BLOCKS_NO_LOOPS
    };
    let is_static_inline_eligible = data.is_static_inline
        && (inst_count <= MAX_INLINE_INSTRUCTIONS
            || (data.has_vector_intrinsics && inst_count <= MAX_VECTOR_STATIC_INLINE_INSTRUCTIONS))
        && data.blocks.len()
            <= if data.has_vector_intrinsics {
                MAX_VECTOR_STATIC_INLINE_BLOCKS
            } else {
                static_inline_block_limit
            };

    let is_gnu_inline_eligible =
        data.is_gnu_inline_def && inst_count <= 128 && data.blocks.len() <= 16;
    data.is_always_inline
        || is_tiny
        || is_small
        || is_static_inline_eligible
        || is_gnu_inline_eligible
}

/// Estimate a callee's instruction count after mandatory descendants expand.
/// Replacing a direct call removes one instruction, hence `child_cost - 1`.
/// Cycles and unusually deep wrapper chains are bounded conservatively.
fn estimate_size_inline_cost(
    name: &str,
    map: &FxHashMap<String, CalleeData>,
    visiting: &mut FxHashSet<String>,
    depth: usize,
) -> usize {
    let Some(data) = map.get(name) else {
        return 0;
    };
    let raw_count: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
    if depth >= 16 || !visiting.insert(name.to_string()) {
        return raw_count;
    }

    let mut cost = raw_count;
    for block in &data.blocks {
        for inst in &block.instructions {
            let Instruction::Call {
                func: child_name, ..
            } = inst
            else {
                continue;
            };
            let Some(child) = map.get(child_name) else {
                continue;
            };
            if !is_mandatory_first_pass_callee(child) {
                continue;
            }
            let child_cost = estimate_size_inline_cost(child_name, map, visiting, depth + 1);
            cost = cost.saturating_add(child_cost.saturating_sub(1));
        }
    }

    visiting.remove(name);
    cost
}

/// True if the function contains any SIMD vector intrinsics (SSE/AVX/AVX2 ops
/// that operate on xmm/ymm vectors). Used to grant static-inline functions a
/// larger inlining budget, since their un-inlined codegen is memory-bound.
fn func_has_vector_intrinsics(func: &IrFunction) -> bool {
    use crate::ir::intrinsics::IntrinsicOp;
    func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| {
            if let Instruction::Intrinsic { op, .. } = inst {
                let name = format!("{:?}", op);
                name.ends_with("256")
                    || name.ends_with("128")
                    || name.starts_with("VecLoad")
                    || name.starts_with("Loaddqu")
                    || matches!(
                        op,
                        IntrinsicOp::Loadu256
                            | IntrinsicOp::Load256
                            | IntrinsicOp::Storeu256
                            | IntrinsicOp::Store256
                            | IntrinsicOp::Pmovmskb256
                            | IntrinsicOp::Broadcast128to256
                            | IntrinsicOp::Zext128to256
                            | IntrinsicOp::Cast256to128
                            | IntrinsicOp::Insert128to256
                            | IntrinsicOp::Pmovmskb128
                    )
            } else {
                false
            }
        })
    })
}

/// Find call sites in a function that are eligible for inlining.
/// `caller_has_section`: true if the caller has a custom section attribute.
/// When true, callees that exceed normal inline limits (but fit relaxed limits)
/// are also eligible, since cross-section calls from section-attributed functions
/// (e.g., .head.text, .noinstr.text) can cause boot/runtime failures.
fn find_inline_call_sites(
    func: &IrFunction,
    callee_map: &FxHashMap<String, CalleeData>,
    skip_list: &[String],
    caller_has_section: bool,
) -> Vec<InlineCallSite> {
    let mut sites = Vec::with_capacity(16);

    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Call {
                func: callee_name,
                info,
            } = inst
            {
                if let Some(callee_data) = callee_map.get(callee_name) {
                    // Don't inline recursive calls
                    if callee_name != &func.name {
                        // Skip functions listed in CCC_INLINE_SKIP
                        if skip_list.iter().any(|s| s == callee_name) {
                            continue;
                        }
                        // Skip callees that exceed normal limits unless caller has a section
                        if callee_data.exceeds_normal_limits && !caller_has_section {
                            continue;
                        }
                        sites.push(InlineCallSite {
                            block_idx,
                            inst_idx,
                            callee_name: callee_name.clone(),
                            dest: info.dest,
                            args: info.args.clone(),
                            pgo_force: false,
                            arg_meta: CallArgMeta::from_call_info(info),
                        });
                    }
                }
            }
        }
    }

    sites
}

/// Inline a single call site. Returns true if successful.
/// `global_max_block_id` is the module-global max block ID, updated on success.
/// A scalar parameter is SSA-substitutable when its home alloca is never
/// address-taken and never written after the initial ParamRef store. For such
/// parameters the inliner can replace every `Load(home)` with the call
/// argument directly, eliminating the store-into-alloca + load round-trip from
/// inlined code (smaller, faster, and immune to memory-forwarding interactions).
fn param_substitutable(callee: &CalleeData, param_idx: usize, home: Value) -> bool {
    // Struct-by-value parameters use the memcpy path; never substitute.
    if callee
        .param_struct_sizes
        .get(param_idx)
        .copied()
        .flatten()
        .is_some()
    {
        return false;
    }
    let mut home_volatile = false;
    let mut paramref_dest: Option<Value> = None;
    for block in &callee.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest,
                    volatile: true,
                    ..
                } if *dest == home => {
                    home_volatile = true;
                }
                Instruction::ParamRef {
                    dest,
                    param_idx: pi,
                    ..
                } if *pi == param_idx => {
                    paramref_dest = Some(*dest);
                }
                _ => {}
            }
        }
    }
    if home_volatile {
        return false;
    }
    // Params without an explicit ParamRef (e.g. the sret pointer) are filled by
    // the backend's emit_store_params; substituting their loads would change ABI
    // handling. Keep them on the memory path.
    if paramref_dest.is_none() {
        return false;
    }
    for block in &callee.blocks {
        for inst in &block.instructions {
            match inst {
                // Loads from the home are the reads we substitute.
                Instruction::Load { ptr, .. } => {
                    if *ptr == home {
                        continue;
                    }
                }
                // The initial ParamRef store is the one the inliner removes.
                Instruction::Store { val, ptr, .. } => {
                    if *ptr == home {
                        let is_initial =
                            matches!(val, Operand::Value(v) if Some(*v) == paramref_dest);
                        if !is_initial {
                            return false; // modified after init
                        }
                        continue;
                    }
                }
                _ => {}
            }
            // Any other reference to the home alloca is an address escape.
            let mut escapes = false;
            inst.for_each_used_value(|id| {
                if id == home.0 {
                    escapes = true;
                }
            });
            if escapes {
                return false;
            }
        }
    }
    true
}

/// IR type of a caller-side operand; needed to substitute call args at the
/// parameter width (a raw Copy at the arg's own width would corrupt
/// adjacent stack slots where the store/load path truncated implicitly).
fn const_ir_type(c: &IrConst) -> Option<IrType> {
    Some(match c {
        IrConst::I8(_) => IrType::I8,
        IrConst::I16(_) => IrType::I16,
        IrConst::I32(_) => IrType::I32,
        IrConst::I64(_) => IrType::I64,
        IrConst::I128(_) => IrType::I128,
        IrConst::F32(_) => IrType::F32,
        IrConst::F64(_) => IrType::F64,
        IrConst::D32(_) => IrType::D32,
        IrConst::D64(_) => IrType::D64,
        IrConst::LongDouble(..) => IrType::F128,
        IrConst::Zero => crate::common::types::target_int_ir_type(),
    })
}

fn inline_call_site(
    caller: &mut IrFunction,
    site: &InlineCallSite,
    callee: &CalleeData,
    global_max_block_id: &mut u32,
) -> bool {
    if callee.blocks.is_empty() {
        return false;
    }

    // Compute ID offsets for remapping callee values and blocks into caller's
    // namespace. Structural transforms may have introduced values without
    // advancing next_value_id, so it is an optimization hint rather than an
    // authority: always reserve past the actual maximum present in the IR.
    let caller_next_value = std::cmp::max(
        caller.next_value_id,
        caller.max_value_id().saturating_add(1),
    );

    // Use the global max block ID to avoid collisions with ANY function's blocks
    let value_offset = caller_next_value;
    let block_offset = *global_max_block_id + 1;

    let debug_inline_detail = std::env::var("CCC_INLINE_DEBUG_DETAIL").is_ok();
    if debug_inline_detail {
        eprintln!(
            "[INLINE_DETAIL] Inlining '{}' into '{}': value_offset={}, block_offset={}, callee.next_value_id={}, caller.next_value_id={}",
            site.callee_name,
            caller.name,
            value_offset,
            block_offset,
            callee.next_value_id,
            caller.next_value_id
        );
        eprintln!(
            "[INLINE_DETAIL]   site.block_idx={}, site.inst_idx={}",
            site.block_idx, site.inst_idx
        );
        for (i, arg) in site.args.iter().enumerate() {
            eprintln!("[INLINE_DETAIL]   arg[{}] = {:?}", i, arg);
        }
    }

    // Clone and remap the callee's blocks
    let mut inlined_blocks: Vec<BasicBlock> = Vec::with_capacity(callee.blocks.len());

    for callee_block in &callee.blocks {
        let mut new_block = BasicBlock {
            label: BlockId(callee_block.label.0 + block_offset),
            instructions: Vec::with_capacity(callee_block.instructions.len()),
            source_spans: callee_block.source_spans.clone(),
            terminator: remap_terminator(&callee_block.terminator, value_offset, block_offset),
        };

        for inst in &callee_block.instructions {
            new_block
                .instructions
                .push(remap_instruction(inst, value_offset, block_offset));
        }

        inlined_blocks.push(new_block);
    }

    // ── S17: __builtin_va_arg_pack() forwarding ────────────────────────────
    // The wrapper's caller evaluated ALL its arguments (named + variadic)
    // before the call; the cloned body's sentinel call is deleted and the
    // arguments beyond the wrapper's named parameters are spliced into the
    // consuming call's argument list with their full ABI metadata.  A
    // va_arg_pack_len sentinel becomes an I32 constant via Copy, so the
    // regular const-propagation machinery sees the count.
    if let Some(plan) = &callee.va_arg_pack {
        let named = callee.num_params.min(site.args.len());
        let extra: Vec<Operand> = site.args[named..].to_vec();
        let extra_meta = site.arg_meta.slice(named, site.args.len());

        for v in plan.vp_values.clone() {
            let vrem = Value(v.0 + value_offset);
            let vp_name = "__lccc_va_arg_pack";
            for block in &mut inlined_blocks {
                // Delete the sentinel call itself.
                block.instructions.retain(|inst| {
                    !matches!(inst, Instruction::Call { func, info }
                            if func == vp_name && info.dest == Some(vrem))
                });
                // Splice the forwarded arguments at every consuming site.
                for inst in &mut block.instructions {
                    if let Instruction::Call { info, .. } = inst {
                        // Replace the single forwarded-argument slot with the
                        // call site's extra arguments.  Every parallel ABI
                        // array must drop the vp slot's own entry AND take
                        // the extras' entries, keeping it index-aligned with
                        // `args` (an insert-without-removal leaves the arrays
                        // one entry longer than `args`, and every later ABI
                        // classification reads shifted metadata — the wrong
                        // struct slot got memcpy'd, va_start read past the
                        // named args, and the wrapper's own checks aborted).
                        fn splice_slot<T: Clone>(arr: &mut Vec<T>, i: usize, extra: &[T]) {
                            match i.cmp(&arr.len()) {
                                std::cmp::Ordering::Less => {
                                    arr.splice(i..i + 1, extra.iter().cloned());
                                }
                                std::cmp::Ordering::Equal => {
                                    arr.extend(extra.iter().cloned());
                                }
                                std::cmp::Ordering::Greater => {}
                            }
                        }
                        let mut i = 0;
                        while i < info.args.len() {
                            if info.args[i] == Operand::Value(vrem) {
                                info.args.splice(i..i + 1, extra.iter().cloned());
                                splice_slot(&mut info.arg_types, i, &extra_meta.arg_types);
                                splice_slot(
                                    &mut info.struct_arg_sizes,
                                    i,
                                    &extra_meta.struct_arg_sizes,
                                );
                                splice_slot(
                                    &mut info.struct_arg_aligns,
                                    i,
                                    &extra_meta.struct_arg_aligns,
                                );
                                splice_slot(
                                    &mut info.struct_arg_classes,
                                    i,
                                    &extra_meta.struct_arg_classes,
                                );
                                splice_slot(
                                    &mut info.struct_arg_riscv_float_classes,
                                    i,
                                    &extra_meta.struct_arg_riscv_float_classes,
                                );
                                splice_slot(
                                    &mut info.struct_arg_is_f128_sse,
                                    i,
                                    &extra_meta.struct_arg_is_f128_sse,
                                );
                                i += extra.len();
                            } else {
                                i += 1;
                            }
                        }
                    }
                }
            }
        }

        if !plan.len_values.is_empty() && !extra.is_empty() {
            let len_const = IrConst::I32(extra.len() as i32);
            for block in &mut inlined_blocks {
                for inst in &mut block.instructions {
                    if let Instruction::Call { func, info } = inst {
                        if func == "__lccc_va_arg_pack_len" {
                            if let Some(d) = info.dest {
                                if plan.len_values.contains(&Value(d.0 + value_offset)) {
                                    // Rewrite in place: sentinel call -> Copy of the count.
                                    *inst = Instruction::Copy {
                                        dest: Value(d.0 + value_offset),
                                        src: Operand::Const(len_const.clone()),
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Create a merge block that the callee's return statements will branch to
    let merge_block_id = BlockId(block_offset + callee.max_block_id + 1);

    // Collect return values from all return blocks to build a Phi node.
    // Each Return(Some(val)) becomes a branch to the merge block, and
    // the return value feeds into a Phi in the merge block.
    let mut phi_incoming: Vec<(Operand, BlockId)> = Vec::new();

    // Replace Return terminators in inlined blocks
    for block in &mut inlined_blocks {
        if let Terminator::Return(ret_val) = &block.terminator {
            if let (Some(_call_dest), Some(ret_operand)) = (site.dest, ret_val) {
                phi_incoming.push((*ret_operand, block.label));
            }
            block.terminator = Terminator::Branch(merge_block_id);
        }
    }

    // Now we need to wire up the arguments. The callee's first N allocas are parameter allocas.
    // We need to store the caller's arguments into those allocas.
    // The param allocas are the first N Alloca instructions in the callee's entry block.
    let entry_block = &mut inlined_blocks[0];
    let mut param_alloca_info: Vec<(Value, IrType, usize)> = Vec::new(); // (dest, ty, size)
    for inst in &entry_block.instructions {
        if let Instruction::Alloca { dest, ty, size, .. } = inst {
            param_alloca_info.push((*dest, *ty, *size));
            if param_alloca_info.len() >= callee.num_params {
                break;
            }
        }
    }

    // ms178: SSA parameter substitution. For pure scalar params (home never
    // address-taken, never re-stored) a constant argument replaces every home
    // load, skipping the store+load round-trip; the home alloca is dropped.
    // Restricted to CONSTANT args: a runtime arg is an SSA value whose live
    // range crosses the call site, and LCCC's linear-scan allocator can leave
    // such a value in an undefined callee-saved register on some paths
    // (zlib-ng deflateSetParamPre *out=param wrote a stale %rbx). Constants
    // are materialized at the use site, so they have no register interaction.
    let orig_homes: Vec<Value> = {
        let mut v = Vec::with_capacity(16);
        for inst in &callee.blocks[0].instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                v.push(*dest);
                if v.len() >= callee.num_params {
                    break;
                }
            }
        }
        v
    };
    // Record remapped ParamRef values before removing them.  Unlike ordinary
    // home loads, ParamRef is an SSA incoming parameter value; it can survive
    // prior structural transforms and be consumed directly by intrinsics.
    // Every such value must receive a caller-defined replacement before its
    // instruction is removed.
    let mut paramref_records: Vec<(Value, usize, IrType)> = Vec::new();
    let mut paramref_params: crate::common::fx_hash::FxHashSet<usize> =
        crate::common::fx_hash::FxHashSet::default();
    for block in &inlined_blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest,
                param_idx,
                ty,
            } = inst
            {
                paramref_records.push((*dest, *param_idx, *ty));
                paramref_params.insert(*param_idx);
            }
        }
    }

    // CCC_NO_SSA_PARAM=1 disables parameter substitution (diagnostic toggle).
    let ssa_param_enabled = std::env::var("CCC_NO_SSA_PARAM").is_err();
    // CCC_SSA_PARAM_SKIP=callee1,callee2 disables substitution for named
    // callees; CCC_SSA_PARAM_LOG=1 traces substitutions (diagnostics).
    let ssa_skip: crate::common::fx_hash::FxHashSet<String> = std::env::var("CCC_SSA_PARAM_SKIP")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let ssa_log = std::env::var("CCC_SSA_PARAM_LOG").is_ok();
    let substitutable: Vec<bool> = (0..param_alloca_info.len())
        .map(|i| {
            let ok = ssa_param_enabled
                && !ssa_skip.contains(&site.callee_name)
                && i < site.args.len()
                // ParamRef users are materialized below from their caller
                // store. Do not remove their home alloca through the older
                // load-only optimization.
                && !paramref_params.contains(&i)
                && orig_homes.get(i).is_some_and(|h| param_substitutable(callee, i, *h));
            if ok && ssa_log {
                eprintln!("[SSA_PARAM] {} arg{} substituted", site.callee_name, i);
            }
            ok
        })
        .collect();
    // home_subst: home id -> (arg, arg_ir_type). Substitution is restricted
    // to COMPILE-TIME CONSTANT arguments: a substituted runtime value is an
    // SSA value whose live range crosses the call site (and often loop
    // boundaries); LCCC's linear-scan allocator can then assign it a
    // callee-saved register that is never defined on some path (regression:
    // zlib-ng deflateSetParamPre *out=param wrote a stale %rbx). Constants
    // are materialized at the use site, so they have no register/liveness
    // interaction and are unconditionally safe.
    let mut home_subst: FxInlineMap<u32, (Operand, IrType)> = FxInlineMap::default();
    for i in 0..param_alloca_info.len() {
        if substitutable[i] {
            if let Operand::Const(c) = &site.args[i] {
                if let Some(at) = const_ir_type(c) {
                    home_subst.insert(param_alloca_info[i].0 .0, (site.args[i], at));
                }
            }
        }
    }

    // Insert stores/memcpys of arguments into param allocas at the beginning of the
    // entry block (after the allocas themselves). ParamRef materializations are
    // inserted immediately after this argument bridge so they are defined on
    // every inlined path before any cloned block can consume them.
    let mut paramref_next_value = value_offset + callee.next_value_id;
    let mut paramref_subst: FxInlineMap<u32, Operand> = FxInlineMap::default();
    let paramref_debug = std::env::var("CCC_INLINE_PARAMREF_DEBUG").is_ok();
    {
        let entry_block = &mut inlined_blocks[0];
        let mut insert_pos = 0;
        // Find position after all allocas in the entry block.
        for (i, inst) in entry_block.instructions.iter().enumerate() {
            if matches!(inst, Instruction::Alloca { .. }) {
                insert_pos = i + 1;
            } else {
                break;
            }
        }

        // Insert stores in reverse order so indices stay valid. Every parameter
        // with a live ParamRef retains this bridge even for a constant argument:
        // constants may replace the ParamRef directly, while the home remains a
        // valid ABI object for address-taken or later memory uses.
        let has_spans = !entry_block.source_spans.is_empty();
        let num_args_to_store = std::cmp::min(site.args.len(), param_alloca_info.len());
        let mut inserted_arg_count = 0usize;
        for i in (0..num_args_to_store).rev() {
            if home_subst.contains_key(&param_alloca_info[i].0 .0) {
                continue; // value flows via the older pure-home substitution
            }
            let param_struct_size = callee.param_struct_sizes.get(i).copied().flatten();
            if let Some(struct_size) = param_struct_size {
                // Struct-by-value parameter: the caller passes a pointer to the struct data.
                // We must copy the struct data from that pointer into the callee's param alloca.
                if let Operand::Value(src_ptr) = site.args[i] {
                    entry_block.instructions.insert(
                        insert_pos,
                        Instruction::Memcpy {
                            dest: param_alloca_info[i].0,
                            src: src_ptr,
                            size: struct_size,
                        },
                    );
                    if has_spans {
                        entry_block
                            .source_spans
                            .insert(insert_pos, crate::common::source::Span::dummy());
                    }
                    inserted_arg_count += 1;
                } else {
                    // Struct arg should always be a Value (pointer), not a Const.
                    return false;
                }
            } else {
                let store_ty = param_alloca_info[i].1;
                entry_block.instructions.insert(
                    insert_pos,
                    Instruction::Store {
                        volatile: false,
                        val: site.args[i],
                        ptr: param_alloca_info[i].0,
                        ty: store_ty,
                        seg_override: AddressSpace::Default,
                    },
                );
                if has_spans {
                    entry_block
                        .source_spans
                        .insert(insert_pos, crate::common::source::Span::dummy());
                }
                inserted_arg_count += 1;
            }
        }

        let mut materialize_pos = insert_pos + inserted_arg_count;
        // Deduplicate loads: one Load per distinct param_idx, and one Cast per
        // distinct (param_idx, param_ty) when types differ.  This reduces IR
        // bloat when a callee has multiple ParamRef for same parameter (common
        // after TCE / loop lowering) and keeps compile-time low.
        use crate::common::fx_hash::FxHashMap as ParamMap;
        let mut loaded_per_param: ParamMap<usize, Value> = ParamMap::default();
        let mut cast_per_param_ty: ParamMap<(usize, IrType), Value> = ParamMap::default();

        for (paramref_dest, param_idx, param_ty) in &paramref_records {
            if *param_idx >= num_args_to_store || *param_idx >= param_alloca_info.len() {
                return false;
            }

            let (home, home_ty, _) = param_alloca_info[*param_idx];

            // Reuse existing load for this param_idx if present
            let loaded = if let Some(&v) = loaded_per_param.get(param_idx) {
                v
            } else {
                let v = Value(paramref_next_value);
                paramref_next_value += 1;
                entry_block.instructions.insert(
                    materialize_pos,
                    Instruction::Load {
                        volatile: false,
                        dest: v,
                        ptr: home,
                        ty: home_ty,
                        seg_override: AddressSpace::Default,
                    },
                );
                if has_spans {
                    entry_block
                        .source_spans
                        .insert(materialize_pos, crate::common::source::Span::dummy());
                }
                materialize_pos += 1;
                loaded_per_param.insert(*param_idx, v);
                v
            };

            let replacement = if home_ty == *param_ty {
                Operand::Value(loaded)
            } else {
                let key = (*param_idx, *param_ty);
                if let Some(&casted) = cast_per_param_ty.get(&key) {
                    Operand::Value(casted)
                } else {
                    let cast = Value(paramref_next_value);
                    paramref_next_value += 1;
                    entry_block.instructions.insert(
                        materialize_pos,
                        Instruction::Cast {
                            dest: cast,
                            src: Operand::Value(loaded),
                            from_ty: home_ty,
                            to_ty: *param_ty,
                        },
                    );
                    if has_spans {
                        entry_block
                            .source_spans
                            .insert(materialize_pos, crate::common::source::Span::dummy());
                    }
                    materialize_pos += 1;
                    cast_per_param_ty.insert(key, cast);
                    Operand::Value(cast)
                }
            };
            if paramref_debug {
                eprintln!(
                    "[INLINE_PARAMREF] {} param{} remapped v{} -> {:?}",
                    site.callee_name, param_idx, paramref_dest.0, replacement
                );
            }
            paramref_subst.insert(paramref_dest.0, replacement);
        }
    }

    // Remove ParamRef instructions from inlined blocks. After inlining, ParamRef
    // instructions are invalid because they reference param_idx of the callee,
    // but at codegen time they would be interpreted as param_idx of the caller.
    // The inliner already handles argument passing via stores above, so ParamRef
    // instructions (and their associated stores to param allocas) are redundant.
    // We also remove the Store that immediately follows each ParamRef since it
    // stores the (now-removed) ParamRef dest into a param alloca.
    let param_alloca_set: crate::common::fx_hash::FxHashSet<u32> =
        param_alloca_info.iter().map(|(v, _, _)| v.0).collect();
    for block in &mut inlined_blocks {
        let has_spans =
            block.source_spans.len() == block.instructions.len() && !block.source_spans.is_empty();
        let old_spans = std::mem::take(&mut block.source_spans);
        let mut new_insts = Vec::with_capacity(block.instructions.len());
        let mut new_spans = Vec::with_capacity(16);
        let mut paramref_dests: crate::common::fx_hash::FxHashSet<u32> =
            crate::common::fx_hash::FxHashSet::default();
        for (idx, inst) in block.instructions.drain(..).enumerate() {
            if let Instruction::ParamRef { dest, .. } = &inst {
                paramref_dests.insert(dest.0);
                // Don't emit this instruction; mark that the next Store to a param alloca should be skipped
                continue;
            }
            // Skip stores of ParamRef dests to param allocas
            if let Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ..
            } = &inst
            {
                if paramref_dests.contains(&v.0) && param_alloca_set.contains(&ptr.0) {
                    continue;
                }
            }
            // ms178: SSA parameter substitution — replace loads from a pure
            // scalar param home with the call argument, and drop the now-dead
            // home alloca.
            if let Instruction::Alloca { dest, .. } = &inst {
                if home_subst.contains_key(&dest.0) {
                    continue;
                }
            }
            if let Instruction::Load { dest, ptr, ty, .. } = &inst {
                if let Some(&(arg, at)) = home_subst.get(&ptr.0) {
                    if at == *ty {
                        new_insts.push(Instruction::Copy {
                            dest: *dest,
                            src: arg,
                        });
                    } else {
                        new_insts.push(Instruction::Cast {
                            dest: *dest,
                            src: arg,
                            from_ty: at,
                            to_ty: *ty,
                        });
                    }
                    if has_spans {
                        new_spans.push(old_spans[idx]);
                    }
                    continue;
                }
            }
            new_insts.push(inst);
            if has_spans {
                new_spans.push(old_spans[idx]);
            }
        }
        block.instructions = new_insts;
        if has_spans {
            block.source_spans = new_spans;
        }
    }
    // Replace every direct ParamRef use after removing ParamRef instructions.
    // This includes intrinsic arguments, phi incoming values, and terminators;
    // leaving even one remapped ParamRef unbound produces an invalid backend
    // value with no register or stack home.
    if !paramref_subst.is_empty() {
        for block in &mut inlined_blocks {
            for inst in &mut block.instructions {
                inst.for_each_operand_mut(|op| {
                    if let Operand::Value(v) = op {
                        if let Some(&replacement) = paramref_subst.get(&v.0) {
                            *op = replacement;
                        }
                    }
                });
                inst.for_each_value_use_mut(|value| {
                    if let Some(Operand::Value(replacement)) = paramref_subst.get(&value.0) {
                        if paramref_debug {
                            eprintln!(
                                "[INLINE_PARAMREF] {} direct v{} -> v{}",
                                site.callee_name, value.0, replacement.0
                            );
                        }
                        *value = *replacement;
                    }
                });
            }
            block.terminator.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    if let Some(&replacement) = paramref_subst.get(&v.0) {
                        *op = replacement;
                    }
                }
            });
        }
        // CRITICAL: phi_incoming was collected from the callee's Return
        // terminators BEFORE this substitution ran (those Returns are
        // already rewritten to Branch(merge), so the loop above cannot see
        // their operands). A callee that returns its own parameter (e.g.
        // expat's checkCharRefNumber: `return result` where result IS the
        // ParamRef value after copy-prop) left the stale pre-substitution
        // id in the merge phi — an undefined value at codegen time (the
        // switch 'break' path returned 0 instead of result; char refs
        // &#60; etc. all decoded to 0).
        for (op, _) in phi_incoming.iter_mut() {
            if let Operand::Value(v) = op {
                if let Some(&replacement) = paramref_subst.get(&v.0) {
                    if paramref_debug {
                        eprintln!(
                            "[INLINE_PARAMREF] {} return-phi v{} -> {:?}",
                            site.callee_name, v.0, replacement
                        );
                    }
                    *op = replacement;
                }
            }
        }
    }

    // Now split the caller's block at the call site:
    // Block before call -> instructions before the call + branch to callee entry
    // Block after call (merge block) -> instructions after the call + original terminator

    let call_block_idx = site.block_idx;
    let call_inst_idx = site.inst_idx;

    // Save instructions after the call and the terminator
    let after_call_instructions: Vec<Instruction> = caller.blocks[call_block_idx]
        .instructions
        .split_off(call_inst_idx + 1);
    let after_call_spans: Vec<crate::common::source::Span> = {
        let spans = &mut caller.blocks[call_block_idx].source_spans;
        if spans.len() > call_inst_idx + 1 {
            spans.split_off(call_inst_idx + 1)
        } else {
            Vec::new()
        }
    };
    let original_terminator = std::mem::replace(
        &mut caller.blocks[call_block_idx].terminator,
        Terminator::Branch(inlined_blocks[0].label),
    );

    // Remove the call instruction itself
    caller.blocks[call_block_idx].instructions.pop();
    if !caller.blocks[call_block_idx].source_spans.is_empty() {
        caller.blocks[call_block_idx].source_spans.pop();
    }

    // Create the merge block with the remaining instructions and original terminator.
    // If the callee had a non-void return, insert a Phi (or Copy for single-predecessor)
    // at the start of the merge block to define the call's result value.
    let mut merge_instructions = Vec::with_capacity(16);
    let mut merge_spans: Vec<crate::common::source::Span> = Vec::new();
    if let Some(call_dest) = site.dest {
        if phi_incoming.len() == 1 {
            // Single return path: just copy the value directly (no phi needed)
            merge_instructions.push(Instruction::Copy {
                dest: call_dest,
                src: phi_incoming[0].0,
            });
            merge_spans.push(crate::common::source::Span::dummy());
        } else if phi_incoming.len() > 1 {
            // Multiple return paths: need a Phi node
            merge_instructions.push(Instruction::Phi {
                dest: call_dest,
                ty: callee.return_type,
                incoming: phi_incoming,
            });
            merge_spans.push(crate::common::source::Span::dummy());
        }
        // If phi_incoming is empty, the callee never returns a value (e.g., all paths
        // are noreturn/unreachable). The call_dest will be undefined, which is fine
        // since it won't be used.
    }
    merge_instructions.extend(after_call_instructions);
    merge_spans.extend(after_call_spans);

    let merge_block = BasicBlock {
        label: merge_block_id,
        instructions: merge_instructions,
        source_spans: merge_spans,
        terminator: original_terminator,
    };

    // Insert the inlined blocks and merge block after the call block
    let insert_position = call_block_idx + 1;
    // Insert merge block first, then inlined blocks before it
    caller.blocks.insert(insert_position, merge_block);
    for (i, block) in inlined_blocks.into_iter().enumerate() {
        caller.blocks.insert(insert_position + i, block);
    }

    // Update Phi nodes: the original caller block was split at the call site.
    // The merge block inherited the original block's terminator (and thus its
    // successors). Any Phi node in a successor block that references the original
    // split block as an incoming predecessor must now reference the merge block
    // instead, since control flow from the split block now goes through the
    // inlined code and arrives at the successor via the merge block.
    let split_block_label = caller.blocks[call_block_idx].label;
    for block in &mut caller.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_operand, block_id) in incoming.iter_mut() {
                    if *block_id == split_block_label {
                        *block_id = merge_block_id;
                    }
                }
            }
        }
    }

    // Update caller's next_value_id to account for the new values
    let new_next_value_id = std::cmp::max(value_offset + callee.next_value_id, paramref_next_value);
    caller.next_value_id = std::cmp::max(new_next_value_id, caller.next_value_id);
    if debug_inline_detail {
        eprintln!(
            "[INLINE_DETAIL]   after inline: caller.next_value_id={}",
            caller.next_value_id
        );
    }

    // Update the global max block ID so subsequent inlines use fresh IDs.
    // The merge block has the highest ID we assigned.
    *global_max_block_id = merge_block_id.0;

    // The caller's label namespace must be advanced past every block we just
    // cloned. Without this, a later pass that allocates fresh block labels
    // from func.next_label (vectorizer, unroller, ...) reuses the inlined
    // blocks' labels, corrupting CFG edges — observed as an infinite
    // self-loop / out-of-bounds read in the lea_sib_fold -O2 regression
    // (exit CondBranch retargeted onto the loop header).
    caller.next_label = std::cmp::max(caller.next_label, merge_block_id.0 + 1);

    true
}

/// Remap a Value by adding an offset.
fn remap_value(v: Value, offset: u32) -> Value {
    Value(v.0 + offset)
}

/// Remap a BlockId by adding an offset.
fn remap_block(b: BlockId, offset: u32) -> BlockId {
    BlockId(b.0 + offset)
}

/// Remap an Operand (only Value operands need remapping; constants stay the same).
fn remap_operand(op: &Operand, value_offset: u32) -> Operand {
    match op {
        Operand::Value(v) => Operand::Value(remap_value(*v, value_offset)),
        Operand::Const(c) => Operand::Const(*c),
    }
}

/// Remap all values in a CallInfo (shared between Call and CallIndirect remapping).
fn remap_call_info(info: &CallInfo, vo: u32) -> CallInfo {
    CallInfo {
        dest: info.dest.map(|v| remap_value(v, vo)),
        args: info.args.iter().map(|a| remap_operand(a, vo)).collect(),
        arg_types: info.arg_types.clone(),
        return_type: info.return_type,
        is_variadic: info.is_variadic,
        num_fixed_args: info.num_fixed_args,
        struct_arg_sizes: info.struct_arg_sizes.clone(),
        struct_arg_aligns: info.struct_arg_aligns.clone(),
        struct_arg_classes: info.struct_arg_classes.clone(),
        struct_arg_riscv_float_classes: info.struct_arg_riscv_float_classes.clone(),
        // MUST be cloned, not cleared: an inlined _Float128 soft-float call
        // (e.g. __extenddftf2) keeps its single-XMM return convention and its
        // 16-byte XMM argument markers. Clearing them re-classified the
        // inlined call as an [Sse,Sse] i128-style return (xmm0:xmm1 pair),
        // silently corrupting the binary128 payload.
        struct_arg_is_f128_sse: info.struct_arg_is_f128_sse.clone(),
        is_sret: info.is_sret,
        is_fastcall: info.is_fastcall,
        is_pure: info.is_pure,
        is_const: info.is_const,
        ret_eightbyte_classes: info.ret_eightbyte_classes.clone(),
        ret_is_f128_sse: info.ret_is_f128_sse,
    }
}

/// Remap all values and block references in an instruction.
fn remap_instruction(inst: &Instruction, vo: u32, bo: u32) -> Instruction {
    match inst {
        Instruction::PgoCounterInc {
            name,
            offset,
            atomic,
        } => Instruction::PgoCounterInc {
            name: name.clone(),
            offset: *offset,
            atomic: *atomic,
        },
        Instruction::Alloca {
            dest,
            ty,
            size,
            align,
            volatile,
            semantic_volatile,
        } => Instruction::Alloca {
            dest: remap_value(*dest, vo),
            ty: *ty,
            size: *size,
            align: *align,
            volatile: *volatile,
            semantic_volatile: *semantic_volatile,
        },
        Instruction::DynAlloca { dest, size, align } => Instruction::DynAlloca {
            dest: remap_value(*dest, vo),
            size: remap_operand(size, vo),
            align: *align,
        },
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            volatile,
        } => Instruction::Store {
            volatile: *volatile,
            val: remap_operand(val, vo),
            ptr: remap_value(*ptr, vo),
            ty: *ty,
            seg_override: *seg_override,
        },
        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
            volatile,
        } => Instruction::Load {
            volatile: *volatile,
            dest: remap_value(*dest, vo),
            ptr: remap_value(*ptr, vo),
            ty: *ty,
            seg_override: *seg_override,
        },
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => Instruction::BinOp {
            dest: remap_value(*dest, vo),
            op: *op,
            lhs: remap_operand(lhs, vo),
            rhs: remap_operand(rhs, vo),
            ty: *ty,
        },
        Instruction::UnaryOp { dest, op, src, ty } => Instruction::UnaryOp {
            dest: remap_value(*dest, vo),
            op: *op,
            src: remap_operand(src, vo),
            ty: *ty,
        },
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => Instruction::Cmp {
            dest: remap_value(*dest, vo),
            op: *op,
            lhs: remap_operand(lhs, vo),
            rhs: remap_operand(rhs, vo),
            ty: *ty,
        },
        Instruction::Call { func, info } => Instruction::Call {
            func: func.clone(),
            info: remap_call_info(info, vo),
        },
        Instruction::CallIndirect { func_ptr, info } => Instruction::CallIndirect {
            func_ptr: remap_operand(func_ptr, vo),
            info: remap_call_info(info, vo),
        },
        Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty,
        } => Instruction::GetElementPtr {
            dest: remap_value(*dest, vo),
            base: remap_value(*base, vo),
            offset: remap_operand(offset, vo),
            ty: *ty,
        },
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => Instruction::Cast {
            dest: remap_value(*dest, vo),
            src: remap_operand(src, vo),
            from_ty: *from_ty,
            to_ty: *to_ty,
        },
        Instruction::Copy { dest, src } => Instruction::Copy {
            dest: remap_value(*dest, vo),
            src: remap_operand(src, vo),
        },
        Instruction::GlobalAddr { dest, name } => Instruction::GlobalAddr {
            dest: remap_value(*dest, vo),
            name: name.clone(),
        },
        Instruction::Memcpy { dest, src, size } => Instruction::Memcpy {
            dest: remap_value(*dest, vo),
            src: remap_value(*src, vo),
            size: *size,
        },
        Instruction::VaArg {
            dest,
            va_list_ptr,
            result_ty,
        } => Instruction::VaArg {
            dest: remap_value(*dest, vo),
            va_list_ptr: remap_value(*va_list_ptr, vo),
            result_ty: *result_ty,
        },
        Instruction::VaStart { va_list_ptr } => Instruction::VaStart {
            va_list_ptr: remap_value(*va_list_ptr, vo),
        },
        Instruction::VaEnd { va_list_ptr } => Instruction::VaEnd {
            va_list_ptr: remap_value(*va_list_ptr, vo),
        },
        Instruction::VaCopy { dest_ptr, src_ptr } => Instruction::VaCopy {
            dest_ptr: remap_value(*dest_ptr, vo),
            src_ptr: remap_value(*src_ptr, vo),
        },
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            size,
            align,
            ref eightbyte_classes,
        } => Instruction::VaArgStruct {
            dest_ptr: remap_value(*dest_ptr, vo),
            va_list_ptr: remap_value(*va_list_ptr, vo),
            size: *size,
            align: *align,
            eightbyte_classes: eightbyte_classes.clone(),
        },
        Instruction::AtomicRmw {
            dest,
            op,
            ptr,
            val,
            ty,
            ordering,
        } => Instruction::AtomicRmw {
            dest: remap_value(*dest, vo),
            op: *op,
            ptr: remap_operand(ptr, vo),
            val: remap_operand(val, vo),
            ty: *ty,
            ordering: *ordering,
        },
        Instruction::AtomicInc {
            ptr,
            offset,
            ty,
            ordering,
        } => Instruction::AtomicInc {
            ptr: remap_operand(ptr, vo),
            offset: *offset,
            ty: *ty,
            ordering: *ordering,
        },
        Instruction::AtomicCmpxchg {
            dest,
            ptr,
            expected,
            desired,
            ty,
            success_ordering,
            failure_ordering,
            returns_bool,
        } => Instruction::AtomicCmpxchg {
            dest: remap_value(*dest, vo),
            ptr: remap_operand(ptr, vo),
            expected: remap_operand(expected, vo),
            desired: remap_operand(desired, vo),
            ty: *ty,
            success_ordering: *success_ordering,
            failure_ordering: *failure_ordering,
            returns_bool: *returns_bool,
        },
        Instruction::AtomicLoad {
            dest,
            ptr,
            ty,
            ordering,
        } => Instruction::AtomicLoad {
            dest: remap_value(*dest, vo),
            ptr: remap_operand(ptr, vo),
            ty: *ty,
            ordering: *ordering,
        },
        Instruction::AtomicStore {
            ptr,
            val,
            ty,
            ordering,
        } => Instruction::AtomicStore {
            ptr: remap_operand(ptr, vo),
            val: remap_operand(val, vo),
            ty: *ty,
            ordering: *ordering,
        },
        Instruction::Fence { ordering } => Instruction::Fence {
            ordering: *ordering,
        },
        Instruction::Phi { dest, ty, incoming } => Instruction::Phi {
            dest: remap_value(*dest, vo),
            ty: *ty,
            incoming: incoming
                .iter()
                .map(|(op, bid)| (remap_operand(op, vo), remap_block(*bid, bo)))
                .collect(),
        },
        Instruction::LabelAddr { dest, label } => Instruction::LabelAddr {
            dest: remap_value(*dest, vo),
            // A label address is local to the cloned callee body just like a
            // branch target. Keeping the original BlockId made every inline
            // clone point at another function/clone's label (or an undefined
            // .LBB symbol), violating GCC's per-clone label identity.
            label: remap_block(*label, bo),
        },
        Instruction::GetReturnF64Second { dest } => Instruction::GetReturnF64Second {
            dest: remap_value(*dest, vo),
        },
        Instruction::SetReturnF64Second { src } => Instruction::SetReturnF64Second {
            src: remap_operand(src, vo),
        },
        Instruction::GetReturnF32Second { dest } => Instruction::GetReturnF32Second {
            dest: remap_value(*dest, vo),
        },
        Instruction::SetReturnF32Second { src } => Instruction::SetReturnF32Second {
            src: remap_operand(src, vo),
        },
        Instruction::GetReturnF128Second { dest } => Instruction::GetReturnF128Second {
            dest: remap_value(*dest, vo),
        },
        Instruction::SetReturnF128Second { src } => Instruction::SetReturnF128Second {
            src: remap_operand(src, vo),
        },
        // Nested-function support instructions. The inliner never inlines
        // nested functions (they are marked noinline), so these can only
        // appear when inlining a PARENT that calls a nested child; remap
        // their operands and pass the rest through unchanged.
        Instruction::GetStaticChain { dest } => Instruction::GetStaticChain {
            dest: remap_value(*dest, vo),
        },
        Instruction::SetStaticChain { src } => Instruction::SetStaticChain {
            src: remap_operand(src, vo),
        },
        Instruction::InitTrampoline {
            buffer,
            chain,
            func,
        } => Instruction::InitTrampoline {
            buffer: remap_value(*buffer, vo),
            chain: remap_operand(chain, vo),
            func: func.clone(),
        },
        Instruction::NonlocalGotoSave {
            frame,
            rbp_off,
            rsp_off,
        } => Instruction::NonlocalGotoSave {
            frame: remap_value(*frame, vo),
            rbp_off: *rbp_off,
            rsp_off: *rsp_off,
        },
        Instruction::NonlocalGoto {
            chain,
            up,
            rbp_off,
            rsp_off,
            label,
        } => Instruction::NonlocalGoto {
            chain: remap_operand(chain, vo),
            up: *up,
            rbp_off: *rbp_off,
            rsp_off: *rsp_off,
            label: label.clone(),
        },
        Instruction::InlineAsm {
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
            seg_overrides,
        } => Instruction::InlineAsm {
            template: template.clone(),
            outputs: outputs
                .iter()
                .map(|(c, v, n)| (c.clone(), remap_value(*v, vo), n.clone()))
                .collect(),
            inputs: inputs
                .iter()
                .map(|(c, op, n)| (c.clone(), remap_operand(op, vo), n.clone()))
                .collect(),
            clobbers: clobbers.clone(),
            operand_types: operand_types.clone(),
            goto_labels: goto_labels
                .iter()
                .map(|(name, bid)| (name.clone(), remap_block(*bid, bo)))
                .collect(),
            input_symbols: input_symbols.clone(),
            seg_overrides: seg_overrides.clone(),
        },
        Instruction::Intrinsic {
            dest,
            op,
            dest_ptr,
            args,
        } => Instruction::Intrinsic {
            dest: dest.map(|v| remap_value(v, vo)),
            op: *op,
            dest_ptr: dest_ptr.map(|v| remap_value(v, vo)),
            args: args.iter().map(|a| remap_operand(a, vo)).collect(),
        },
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } => Instruction::Select {
            dest: remap_value(*dest, vo),
            cond: remap_operand(cond, vo),
            true_val: remap_operand(true_val, vo),
            false_val: remap_operand(false_val, vo),
            ty: *ty,
        },
        Instruction::StackSave { dest } => Instruction::StackSave {
            dest: remap_value(*dest, vo),
        },
        Instruction::StackRestore { ptr } => Instruction::StackRestore {
            ptr: remap_value(*ptr, vo),
        },
        Instruction::ParamRef {
            dest,
            param_idx,
            ty,
        } => Instruction::ParamRef {
            dest: remap_value(*dest, vo),
            param_idx: *param_idx,
            ty: *ty,
        },
    }
}

/// Remap block references in a terminator.
fn remap_terminator(term: &Terminator, vo: u32, bo: u32) -> Terminator {
    match term {
        Terminator::Return(op) => Terminator::Return(op.map(|o| remap_operand(&o, vo))),
        Terminator::Branch(bid) => Terminator::Branch(remap_block(*bid, bo)),
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => Terminator::CondBranch {
            cond: remap_operand(cond, vo),
            true_label: remap_block(*true_label, bo),
            false_label: remap_block(*false_label, bo),
        },
        Terminator::IndirectBranch {
            target,
            possible_targets,
        } => Terminator::IndirectBranch {
            target: remap_operand(target, vo),
            possible_targets: possible_targets
                .iter()
                .map(|b| remap_block(*b, bo))
                .collect(),
        },
        Terminator::Switch {
            val,
            cases,
            default,
            ty,
        } => Terminator::Switch {
            val: remap_operand(val, vo),
            cases: cases
                .iter()
                .map(|&(v, bid)| (v, remap_block(bid, bo)))
                .collect(),
            default: remap_block(*default, bo),
            ty: *ty,
        },
        Terminator::Unreachable => Terminator::Unreachable,
    }
}

/// Debug: dump function IR in a readable text format.
fn dump_function_ir(func: &IrFunction, context: &str) {
    eprintln!("=== IR DUMP {} ===", context);
    eprintln!(
        "function {} (next_value_id={})",
        func.name, func.next_value_id
    );
    for (bi, block) in func.blocks.iter().enumerate() {
        eprintln!("  block[{}] .L{}:", bi, block.label.0);
        for (ii, inst) in block.instructions.iter().enumerate() {
            eprintln!("    [{}] {}", ii, format_instruction(inst));
        }
        eprintln!("    terminator: {}", format_terminator(&block.terminator));
    }
    eprintln!("=== END IR DUMP ===");
}

fn format_operand(op: &Operand) -> String {
    match op {
        Operand::Value(v) => format!("v{}", v.0),
        Operand::Const(c) => format!("{:?}", c),
    }
}

fn format_instruction(inst: &Instruction) -> String {
    match inst {
        Instruction::Alloca {
            dest,
            ty,
            size,
            align,
            ..
        } => {
            format!(
                "v{} = alloca {:?} size={} align={}",
                dest.0, ty, size, align
            )
        }
        Instruction::Store { val, ptr, ty, .. } => {
            format!("store {:?} {} -> v{}", ty, format_operand(val), ptr.0)
        }
        Instruction::Load { dest, ptr, ty, .. } => {
            format!("v{} = load {:?} v{}", dest.0, ty, ptr.0)
        }
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            format!(
                "v{} = {:?} {:?} {}, {}",
                dest.0,
                op,
                ty,
                format_operand(lhs),
                format_operand(rhs)
            )
        }
        Instruction::UnaryOp { dest, op, src, ty } => {
            format!("v{} = {:?} {:?} {}", dest.0, op, ty, format_operand(src))
        }
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            format!(
                "v{} = cmp {:?} {:?} {}, {}",
                dest.0,
                op,
                ty,
                format_operand(lhs),
                format_operand(rhs)
            )
        }
        Instruction::Call { func, info } => {
            let args_str: Vec<String> = info.args.iter().map(format_operand).collect();
            if let Some(d) = info.dest {
                format!("v{} = call {}({})", d.0, func, args_str.join(", "))
            } else {
                format!("call {}({})", func, args_str.join(", "))
            }
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => {
            format!(
                "v{} = cast {:?}->{:?} {}",
                dest.0,
                from_ty,
                to_ty,
                format_operand(src)
            )
        }
        Instruction::Copy { dest, src } => {
            format!("v{} = copy {}", dest.0, format_operand(src))
        }
        Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty,
        } => {
            format!(
                "v{} = gep {:?} v{}, {}",
                dest.0,
                ty,
                base.0,
                format_operand(offset)
            )
        }
        Instruction::Phi { dest, ty, incoming } => {
            let inc_str: Vec<String> = incoming
                .iter()
                .map(|(op, bid)| format!("[{}, .L{}]", format_operand(op), bid.0))
                .collect();
            format!("v{} = phi {:?} {}", dest.0, ty, inc_str.join(", "))
        }
        Instruction::GlobalAddr { dest, name } => {
            format!("v{} = globaladdr @{}", dest.0, name)
        }
        Instruction::Memcpy { dest, src, size } => {
            format!("memcpy v{}, v{}, {}", dest.0, src.0, size)
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } => {
            format!(
                "v{} = select {:?} {}, {}, {}",
                dest.0,
                ty,
                format_operand(cond),
                format_operand(true_val),
                format_operand(false_val)
            )
        }
        _ => format!("{:?}", inst),
    }
}

fn format_terminator(term: &Terminator) -> String {
    match term {
        Terminator::Return(Some(op)) => format!("ret {}", format_operand(op)),
        Terminator::Return(None) => "ret void".to_string(),
        Terminator::Branch(bid) => format!("br .L{}", bid.0),
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            format!(
                "condbr {}, .L{}, .L{}",
                format_operand(cond),
                true_label.0,
                false_label.0
            )
        }
        Terminator::IndirectBranch { target, .. } => {
            format!("indirectbr {}", format_operand(target))
        }
        Terminator::Switch {
            val,
            cases,
            default,
            ..
        } => {
            let cases_str: Vec<String> = cases
                .iter()
                .map(|(v, bid)| format!("{} => .L{}", v, bid.0))
                .collect();
            format!(
                "switch {}, default .L{}, [{}]",
                format_operand(val),
                default.0,
                cases_str.join(", ")
            )
        }
        Terminator::Unreachable => "unreachable".to_string(),
    }
}

#[cfg(test)]
mod inline_limit_tests {
    use super::*;
    #[test]
    fn normal_limit_always_checks_blocks() {
        assert!(!fits_normal_inline_limits(5, 20, false, false, false, 12));
        assert!(fits_normal_inline_limits(5, 3, false, false, false, 12));
    }
    #[test]
    fn medium_limit_is_static_only() {
        assert!(fits_normal_inline_limits(80, 5, true, false, false, 6));
        assert!(!fits_normal_inline_limits(80, 5, false, false, false, 6));
        assert!(!fits_normal_inline_limits(80, 7, true, false, false, 6));
    }
    #[test]
    fn acyclic_static_leaf_with_thirteen_blocks_is_inlineable() {
        // A short source-level predicate with chained `||` conditions can lower
        // to 13 acyclic blocks (the Expat XML-name classification reproducer).
        // It fits the static instruction budget and should not pay a hot call
        // solely because the generic no-loop cap was one block too small.
        assert!(fits_normal_inline_limits(
            32,
            13,
            true,
            false,
            false,
            MAX_INLINE_BLOCKS_NO_LOOPS,
        ));
        assert!(!fits_normal_inline_limits(
            32,
            MAX_INLINE_BLOCKS_NO_LOOPS + 1,
            true,
            false,
            false,
            MAX_INLINE_BLOCKS_NO_LOOPS,
        ));
    }
    #[test]
    fn static_loop_helper_has_own_bounded_inline_limit() {
        // Small loop helpers remain cheap enough to clone at many sites.
        assert!(fits_static_loop_inline_limits(40, 8, 7));
        // Larger loop bodies are admitted only when cloning is bounded.
        assert!(fits_static_loop_inline_limits(128, 16, 2));
        assert!(!fits_static_loop_inline_limits(128, 16, 3));
        assert!(!fits_static_loop_inline_limits(129, 16, 1));
        assert!(!fits_static_loop_inline_limits(128, 17, 1));
    }
    #[test]
    fn vector_static_inline_limit() {
        assert!(fits_normal_inline_limits(150, 12, true, true, true, 6));
        assert!(fits_normal_inline_limits(199, 24, true, true, true, 6));
        assert!(!fits_normal_inline_limits(201, 12, true, true, true, 6));
        assert!(!fits_normal_inline_limits(150, 25, true, true, true, 6));
        assert!(!fits_normal_inline_limits(150, 12, true, true, false, 6));
    }

    fn wrapper_policy_data(block_count: usize) -> CalleeData {
        CalleeData {
            blocks: (0..block_count)
                .map(|index| BasicBlock {
                    label: BlockId(index as u32),
                    instructions: Vec::new(),
                    terminator: Terminator::Return(None),
                    source_spans: Vec::new(),
                })
                .collect(),
            param_struct_sizes: Vec::new(),
            return_type: IrType::Void,
            num_params: 0,
            next_value_id: 0,
            max_block_id: 0,
            is_always_inline: false,
            exceeds_normal_limits: false,
            is_static_inline: false,
            is_gnu_inline_def: false,
            is_plain_static: true,
            direct_call_count: 2,
            has_inlineable_loop_descendant: false,
            has_inlined_calls: false,
            is_single_call_site_static: false,
            has_loops: false,
            is_recursive: false,
            has_vector_intrinsics: false,
            size_inline_cost: 0,
            va_arg_pack: None,
        }
    }

    #[test]
    fn multisite_loop_wrapper_policy_survives_a_later_inline_invocation() {
        let mut fresh = wrapper_policy_data(1);
        fresh.has_inlineable_loop_descendant = true;
        assert!(multisite_loop_wrapper_should_stay_outlined(
            &fresh, 5, false, false, false
        ));
        assert!(!multisite_loop_wrapper_should_stay_outlined(
            &fresh, 5, false, false, true
        ));
        // A later pipeline invocation sees an expanded loop body rather than
        // the original wrapper. It must retain the earlier policy decision.
        let mut expanded = wrapper_policy_data(13);
        expanded.has_loops = true;
        expanded.has_inlined_calls = true;
        assert!(multisite_loop_wrapper_should_stay_outlined(
            &expanded, 101, false, false, false
        ));
        assert!(!multisite_loop_wrapper_should_stay_outlined(
            &expanded, 101, true, false, false
        ));
        assert!(!multisite_loop_wrapper_should_stay_outlined(
            &expanded, 101, false, true, false
        ));
        // An original loop helper has no acquired descendant; it retains the
        // established multi-site loop-inline policy.
        expanded.has_inlined_calls = false;
        assert!(!multisite_loop_wrapper_should_stay_outlined(
            &expanded, 101, false, false, false
        ));
    }

    #[test]
    fn nested_loop_multisite_plain_static_policy_is_o2_only() {
        let mut helper = wrapper_policy_data(7);
        helper.has_loops = true;
        assert!(nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, true, false, false, false
        ));
        assert!(!nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, true, false, false, true
        ));
        assert!(!nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, false, false, false, false
        ));
        assert!(!nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, true, true, false, false
        ));
        assert!(!nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, true, false, true, false
        ));
        helper.direct_call_count = 1;
        helper.is_single_call_site_static = true;
        assert!(!nested_loop_multisite_static_should_stay_outlined(
            &helper, 20, true, false, false, false
        ));
    }

    #[test]
    fn repeated_small_loop_clone_cap_is_narrow_and_respects_overrides() {
        let mut helper = wrapper_policy_data(8);
        helper.has_loops = true;
        helper.direct_call_count = MAX_SMALL_STATIC_LOOP_INLINE_CLONES + 1;
        assert!(repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, false, false, false
        ));
        helper.direct_call_count = MAX_SMALL_STATIC_LOOP_INLINE_CLONES;
        assert!(!repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, false, false, false
        ));
        helper.direct_call_count = MAX_SMALL_STATIC_LOOP_INLINE_CLONES + 1;
        assert!(!repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, false, false, true
        ));
        assert!(!repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, true, false, false
        ));
        assert!(!repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, false, true, false
        ));
        helper.is_plain_static = false;
        assert!(!repeated_small_loop_clone_should_stay_outlined(
            &helper, 27, false, false, false
        ));
    }
}

#[cfg(test)]
mod value_reference_tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::{GlobalInit, IrGlobal, IrModule};
    use crate::ir::reexports::{BasicBlock, Instruction, Terminator};

    fn user_function(instructions: Vec<Instruction>) -> IrFunction {
        IrFunction {
            name: "user".into(),
            return_type: IrType::Void,
            params: vec![],
            blocks: vec![BasicBlock {
                label: BlockId(1),
                instructions,
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_declaration: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            next_value_id: 2,
            fp_expr_tags: Default::default(),
            next_label: 2,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: vec![],
            uses_sret: false,
            is_fastcall: false,
            is_naked: false,
            no_instrument: false,
            global_init_label_blocks: vec![],
            ret_eightbyte_classes: vec![],
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        }
    }

    fn module_with_global_addr_to(target: &str) -> IrModule {
        let mut module = IrModule::default();
        module.functions = vec![user_function(vec![Instruction::GlobalAddr {
            dest: Value(1),
            name: target.into(),
        }])];
        module
    }

    #[test]
    fn global_addr_reference_blocks_single_site_exemption() {
        let module = module_with_global_addr_to("callee");
        let (referenced, asm) = collect_value_referenced_functions(&module);
        assert!(function_referenced_as_value("callee", &referenced, &asm));
        assert!(!function_referenced_as_value("other", &referenced, &asm));
    }

    #[test]
    fn global_initializer_table_reference_counts() {
        let mut module = module_with_global_addr_to("unrelated");
        module.globals = vec![IrGlobal {
            name: "table".into(),
            ty: IrType::Ptr,
            size: 4,
            align: 4,
            init: GlobalInit::GlobalAddr("callee".into()),
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: false,
            is_used: false,
            is_thread_local: false,
        }];
        let (referenced, asm) = collect_value_referenced_functions(&module);
        assert!(function_referenced_as_value("callee", &referenced, &asm));
    }

    #[test]
    fn plain_direct_call_is_not_a_value_reference() {
        let mut module = module_with_global_addr_to("unrelated");
        module.functions[0].blocks[0]
            .instructions
            .push(Instruction::Call {
                func: "callee".into(),
                info: CallInfo::default(),
            });
        let (referenced, asm) = collect_value_referenced_functions(&module);
        // A direct call alone must NOT mark the callee as referenced-as-value:
        // the callee still dies when its only call site is inlined.
        assert!(!function_referenced_as_value("callee", &referenced, &asm));
    }

    #[test]
    fn inline_asm_template_reference_counts() {
        let mut module = module_with_global_addr_to("unrelated");
        module.functions[0].blocks[0]
            .instructions
            .push(Instruction::InlineAsm {
                template: "call callee".into(),
                outputs: Vec::new(),
                inputs: Vec::new(),
                clobbers: Vec::new(),
                operand_types: Vec::new(),
                goto_labels: Vec::new(),
                input_symbols: Vec::new(),
                seg_overrides: Vec::new(),
            });
        let (referenced, asm) = collect_value_referenced_functions(&module);
        assert!(function_referenced_as_value("callee", &referenced, &asm));
    }

    #[test]
    fn inline_asm_input_symbol_reference_counts() {
        // `asm("... %P0 ..." :: "i"(func))` records the symbol in
        // input_symbols — the outlined body must survive this reference.
        let mut module = module_with_global_addr_to("unrelated");
        module.functions[0].blocks[0]
            .instructions
            .push(Instruction::InlineAsm {
                template: "jmp %P0".into(),
                outputs: Vec::new(),
                inputs: Vec::new(),
                clobbers: Vec::new(),
                operand_types: Vec::new(),
                goto_labels: Vec::new(),
                input_symbols: vec![Some("callee".into())],
                seg_overrides: Vec::new(),
            });
        let (referenced, asm) = collect_value_referenced_functions(&module);
        assert!(function_referenced_as_value("callee", &referenced, &asm));
    }
}
