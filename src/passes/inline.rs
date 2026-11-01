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
use crate::passes::round_cap;

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
/// Bounded-tier call-site cap.  A third site was trialled for the Linux
/// find_next_andnot_bit shape (60 insns at three sites) but reverted — it
/// measured neutral at -O2 and slower at -O3, so the two-site cap stays
/// until backend evidence improves:
/// GCC 14.2/16.2 outline it at -O2, and LCCC-side the inline measures as a
/// tie at -O2 (paired A/B n=7 spans 1; layout-averaged k-sweep n=16 splits
/// 9–7) and a 3–8% LOSS at -O3 (k-sweep wins 3–13, p≈0.01) — the oracles
/// inline at -O3 because their backends exploit it, LCCC's does not (yet).
/// Revisit with backend evidence, a dead-body-only budget (an address-taken
/// body keeps the outlined copy, so `inst*(sites-1)` undercounts by `inst`),
/// and a corpus census proving the 3rd site fires nowhere it loses.
const MAX_STATIC_LOOP_INLINE_CALLS: usize = 2;
/// Constant-trip loop helper tier. A plain static helper whose loop exit is
/// compared against a parameter may be cloned at every call site that passes
/// a constant in `MIN_CONST_TRIP_COUNT..=MAX_CONST_TRIP_COUNT` for that
/// parameter, provided the body fits the tier's own size window below. Every
/// other call keeps the general limits above, and `always_inline` or gnu_inline
/// bodies never enter the tier.
///
/// Measured shape: expat's `sip_round(H, 2|4)` (139 IR instructions, 7 sites,
/// two of them inside the `sip24_update` loop). GCC 16.2, Clang 23.1 and ICX
/// all inline every site. LCCC kept four SipHash state words in memory across
/// rounds and ran 25% more Callgrind instructions (198.9M vs 159.3M) than the
/// inlined form.
///
/// The lower bound: with zero or one iteration no state crosses an iteration
/// boundary, so the clone only adds setup code. `reference(1)` in
/// tests/regression/loop_rotate_pressure_gate.c measured +146 Callgrind
/// instructions when it was cloned at its single-iteration site.
///
/// The upper bound covers the measured SipHash counts (2 and 4) with one
/// doubling of headroom. It was not tuned beyond that. Raising it needs a
/// Callgrind screen with scripts/callgrind_corpus.py first.
const MIN_CONST_TRIP_COUNT: i64 = 2;
const MAX_CONST_TRIP_COUNT: i64 = 8;
/// Size window for a constant-trip clone, per copy.
const MAX_CONST_TRIP_INLINE_INSTRUCTIONS: usize = 160;
const MAX_CONST_TRIP_INLINE_BLOCKS: usize = 16;
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

/// Raw-snapshot size limit for an internal-linkage, loop-free callee whose call
/// site is inside a loop and which may bypass the caller-size caps. It is
/// twice the ordinary inline limit: hot bit-writer/hash helpers (43-47 IR
/// instructions in zstd's encoder) fall just above that limit, while every
/// such clone is still charged to MAX_INLINE_BUDGET_PER_CALLER.
const MAX_HOT_LOOP_INLINE_LEAF_INSTRUCTIONS: usize = 2 * MAX_INLINE_INSTRUCTIONS;
/// Shape cap for an SROA-unlocking helper (`is_sroa_unlockable_candidate`).
/// Admission is decided by the per-aggregate cost model in
/// `sroa_unlockable_aliases_of` (cloned instructions <= removed accesses, and
/// within the remaining budget); this only keeps a helper larger than a whole
/// per-caller budget, which that budget could never admit, out of the model.
const MAX_SROA_UNLOCK_HELPER_INSTRUCTIONS: usize = MAX_INLINE_BUDGET_PER_CALLER;

/// Size limits for a declared-`inline` internal callee WITH a loop whose call
/// site is inside a caller loop (see `is_hot_loop_helper_candidate`): zstd's
/// `ZSTD_count` is 142 instructions in 19 blocks after its own helpers are
/// inlined. The limits are the single-call-site/medium-static instruction
/// limit and the vector-intrinsic block limit -- tiers the inliner already
/// accepts for other hot bodies -- not a fit to one function.
const MAX_HOT_LOOP_HELPER_INSTRUCTIONS: usize = 160;
const MAX_HOT_LOOP_HELPER_BLOCKS: usize = 24;

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

/// Resource guard for `__attribute__((always_inline))` callees: GCC inlines
/// them regardless of size (a failed always_inline is a hard error there,
/// never a silent out-of-line call). The MAX_ALWAYS_INLINE_* limits above
/// stay the *relaxed* tier for ordinary statics of section-attributed
/// callers; applying them to always_inline left zstd's FORCE_INLINE_TEMPLATE
/// bodies (`ZSTD_compressBlock_opt_generic`: 803 IR instructions,
/// `ZSTD_compressBlock_lazy_generic`, `HUF_compress1X_usingCTable_internal_body_loop`)
/// out of line, so their constant template arguments (`optLevel`,
/// `searchMethod`, `dictMode`, unroll flags) were never propagated: c-19 ran
/// 1.95x GCC's instruction count. The guard only stops pathological inputs
/// (generated code) from exhausting memory; it is >10x every real body seen.
const ALWAYS_INLINE_GUARD_INSTRUCTIONS: usize = 16_384;
const ALWAYS_INLINE_GUARD_BLOCKS: usize = 4_096;

/// Phase 0 stops draining a caller once mandatory expansion has grown it
/// past this many IR instructions: acyclic always_inline expansion is
/// unmetered, and an adversarial diamond DAG could otherwise multiply.
const PHASE0_CALLER_GUARD_INSTRUCTIONS: usize = 262_144;

/// A callee whose inlining cannot grow a recursive caller's frame: one block,
/// at most `MAX_SMALL_INLINE_INSTRUCTIONS`, no stack object (`Alloca`,
/// `DynAlloca`, non-promotable aggregate bytes) and no call of its own. It
/// contributes only a few short-lived SSA temporaries, while the call it
/// replaces costs a return-address push plus caller-saved spills per
/// recursion level, so per-level stack use does not grow. zstd's
/// `ZSTD_fracWeight` (12-18 IR instructions) inside the self-recursive
/// `ZSTD_litLengthPrice` is the motivating case: GCC inlines it there,
/// while the 5-instruction "tiny" limit left a call in the pricing loop.
fn is_frame_neutral_leaf(data: &CalleeData, inst_count: usize) -> bool {
    data.blocks.len() == 1
        && inst_count <= MAX_SMALL_INLINE_INSTRUCTIONS
        && data.agg_frame_bytes == 0
        && data.blocks[0].instructions.iter().all(|inst| {
            !matches!(
                inst,
                Instruction::Alloca { .. }
                    | Instruction::DynAlloca { .. }
                    | Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
            )
        })
}

/// Names of defined functions that lie on a cycle of the module's direct
/// call graph (self-loops included), by iterative Tarjan SCC in O(V + E).
///
/// Computed over the FULL call graph, not just always_inline functions:
/// later ordinary inlining composes edges (X -> Z -> Y becomes X -> Y), and
/// any cycle such composition can create already exists here, so this set
/// is a sound superset for every round of the run.
fn functions_on_call_cycles(module: &IrModule) -> FxHashSet<String> {
    let index: FxHashMap<&str, usize> = module
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_declaration)
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let succs: Vec<Vec<usize>> = module
        .functions
        .iter()
        .map(|f| {
            let mut out: Vec<usize> = f
                .blocks
                .iter()
                .flat_map(|b| b.instructions.iter())
                .filter_map(|inst| match inst {
                    Instruction::Call { func: callee, .. } => index.get(callee.as_str()).copied(),
                    _ => None,
                })
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();
    let n = succs.len();
    let mut order = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut cyclic = FxHashSet::default();
    for root in 0..n {
        if order[root] != usize::MAX {
            continue;
        }
        // (node, index of the next successor to visit)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        order[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            if let Some(&w) = succs[v].get(*i) {
                *i += 1;
                if order[w] == usize::MAX {
                    order[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(order[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == order[v] {
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                if members.len() > 1 || succs[v].contains(&v) {
                    cyclic.extend(
                        members
                            .into_iter()
                            .map(|m| module.functions[m].name.clone()),
                    );
                }
            }
        }
    }
    cyclic
}

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

thread_local! {
    /// Whether the one-level self-recursion clone runs. Resolved ONCE by the
    /// driver (`passes::mod`, `CCC_NO_SELF_RECURSION_CLONE`) into per-thread
    /// state: the pass pipeline must not read the process environment per call
    /// (`tests/regression/check_env_test_hygiene.sh` ratchets that count), and
    /// a process-global read would make the kill-switch test race its siblings
    /// on cargo's thread pool.
    static SELF_RECURSION_CLONE_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Compilation-scoped trace switch for parameter-reference remapping.
    static PARAMREF_DEBUG_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Functions `self_recursion::inline_self_recursion` already expanded in
    /// the module being compiled. Each self-recursive function is expanded by
    /// ONE mechanism: the clone skips these, so its level never compounds on
    /// the multi-level expansion (whose bounds alone keep the growth sized).
    static SELF_INLINED: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
}

/// Resolve the self-recursion clone kill switch for this thread, and forget
/// the previous module's `self_inline` expansions.
pub(crate) fn set_self_recursion_clone_enabled(enabled: bool) {
    SELF_RECURSION_CLONE_ENABLED.with(|cell| cell.set(enabled));
    SELF_INLINED.with(|set| set.borrow_mut().clear());
}

/// Set the parameter-reference trace switch once for this compilation.
pub(crate) fn set_paramref_debug_enabled(enabled: bool) {
    PARAMREF_DEBUG_ENABLED.with(|cell| cell.set(enabled));
}

/// Suspend or resume the clone within one module without forgetting its
/// `self_inline` expansions (see the call sites in `passes::mod`).
pub(crate) fn suspend_self_recursion_clone(suspended: bool, enabled: bool) {
    SELF_RECURSION_CLONE_ENABLED.with(|cell| cell.set(enabled && !suspended));
}

/// Record a function `self_inline` expanded (see `SELF_INLINED`).
pub(crate) fn note_self_inlined(name: &str) {
    SELF_INLINED.with(|set| {
        set.borrow_mut().insert(name.to_string());
    });
}

fn was_self_inlined(name: &str) -> bool {
    SELF_INLINED.with(|set| set.borrow().contains(name))
}

fn self_recursion_clone_enabled() -> bool {
    SELF_RECURSION_CLONE_ENABLED.with(|cell| cell.get())
}

/// Largest body this pass duplicates when it clones one level of a *directly
/// self-recursive* callee into itself (see `clone_self_recursion_sites`).
/// GCC's recursive inliner is bounded by
/// `--param max-inline-recursive-depth-auto` and by the caller growth caps
/// rather than by a body size; the size gate here is what keeps a single
/// clone from turning a large recursive body into an I-cache resident twice
/// over. `binary_trees`' `make`/`check`/`destroy` (7-14 IR instructions,
/// 2-3 blocks) are the motivating shapes; the cap leaves room for a body
/// three times that size without admitting anything loop-shaped.
const MAX_SELF_CLONE_INSTRUCTIONS: usize = 64;

/// Maximum blocks in a body admitted to the one-level self clone.
const MAX_SELF_CLONE_BLOCKS: usize = 6;

/// Self-call sites expanded per function. A full binary recursion has two;
/// more than this is a wide recursion whose body would be duplicated once
/// per site in one frame, which stops paying for itself.
const MAX_SELF_CLONE_SITES: usize = 3;

/// Upper bound on the caller's size after the clones. Each clone adds one
/// body, so a 2-site clone of a 40-instruction body lands at 120; the cap
/// keeps a pathological shape from multiplying code size.
const MAX_SELF_CLONE_FINAL_INSTRUCTIONS: usize = 192;

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

/// Allocas at least this large are charged to a callee's aggregate-frame cost.
/// Smaller ones are ignored: mem2reg/SROA promotes them to registers, so they
/// cost no stack space after inlining, whereas an aggregate of this size is
/// materialised in the frame at *every* inline site.
const AGG_FRAME_SLOT_BYTES: usize = 16;

/// `-Os`/`-Oz` veto: keep a callee outlined when inlining it would duplicate a
/// large aggregate local frame at several call sites.
///
/// Instruction counts are blind to stack cost. `arch/x86/boot/tty.c`'s
/// `kbd_pending`/`gettime` are ~15-instruction helpers that each materialise a
/// 44-byte `struct biosregs`; inlining them into `getchar_timeout` grew it from
/// GCC's 71 bytes to 290 bytes (4.1x), eating a sixth of the 32 KiB real-mode
/// setup budget on a single function. An outlined body pays the frame once for
/// all its callers, so with more than one call site outlining is the size win --
/// which is exactly what GCC's `-Os` heuristic concludes here.
///
/// Scoped deliberately: callees that *must* be inlined (`always_inline`, and
/// gnu_inline definitions, which have no out-of-line body) are exempt, as are
/// profile-forced hot sites and single-call-site callees (where inlining is
/// size-neutral because the outlined copy disappears).
fn aggregate_frame_should_stay_outlined(
    callee_data: &CalleeData,
    pgo_force: bool,
    size_optimized: bool,
) -> bool {
    size_optimized
        && callee_data.agg_frame_bytes >= AGG_FRAME_SLOT_BYTES
        && !callee_data.is_always_inline
        && !callee_data.is_gnu_inline_def
        && !callee_data.single_call_site
        && !pgo_force
}

/// Select the best call site to inline from the given candidates.
///
/// Uses a two-pass strategy:
/// 1. First pass: pick tiny/small/static-inline callees (always inlined for
///    code correctness, e.g., constant-returning stubs, linker symbol resolution).
/// 2. Second pass: pick the first eligible normal callee that fits within
///    budget and caller size constraints.
///
/// Returns `(site, callee_inst_count, use_relaxed)` or `None` if no eligible site.
/// Size/shape test for an internal-linkage loop-free leaf that may be inlined
/// at an in-loop site despite caller-size caps (and, for declared-inline
/// bodies, despite exceeding the ordinary static-inline limit). The in-loop
/// and budget conditions are decided per site in `select_inline_site`.
/// Plain statics qualify too: GCC auto-inlines small static leaves in hot
/// loops, and devirtualized template predicates (zstd's match finders) are
/// plain statics.
fn is_hot_leaf_candidate(data: &CalleeData) -> bool {
    let instructions: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
    (data.is_static_inline || data.is_plain_static)
        && !data.is_always_inline
        && !data.has_loops
        && !data.is_recursive
        && !data.const_trip_only
        && !data.const_branch_only
        && instructions <= MAX_HOT_LOOP_INLINE_LEAF_INSTRUCTIONS
        && data.blocks.len() <= MAX_INLINE_BLOCKS_NO_LOOPS
}

/// Acyclic internal-linkage helper whose call site passes a pointer to a
/// caller-local aggregate alloca (e.g. `BIT_flushBits(&blockStream)`, or
/// zstd's plain-static `HUF_initCStream(&bitC, ...)` /
/// `HUF_closeCStream(&bitC)`): inlining it is what removes the last
/// address-escape of that alloca so `aggregate_sroa` can split and promote its
/// fields to SSA registers across the loop.
///
/// Declared `inline` is deliberately not required: whether the escape goes
/// away does not depend on the keyword, and GCC auto-inlines such plain
/// statics at -O2 (`-finline-small-functions`). Whether a candidate is worth
/// cloning is decided per aggregate by the cost model in
/// [`sroa_unlockable_aliases_of`], not by this shape test; the
/// [`MAX_SROA_UNLOCK_HELPER_INSTRUCTIONS`] cap only keeps a helper larger than
/// a whole per-caller budget out of that model. `always_inline` helpers are
/// excluded because Phase 0 already drains them; one left behind keeps its
/// aggregate disqualified (conservative).
fn is_sroa_unlockable_candidate(data: &CalleeData) -> bool {
    let instructions: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
    (data.is_static_inline || data.is_plain_static)
        && !data.is_always_inline
        && !data.has_loops
        && !data.is_recursive
        && !data.const_trip_only
        && !data.const_branch_only
        && instructions <= MAX_SROA_UNLOCK_HELPER_INSTRUCTIONS
        && data.blocks.len() <= MAX_INLINE_BLOCKS_NO_LOOPS
}

/// Collect caller values that alias a non-parameter local aggregate alloca
/// (`size <= 64`) whose address only escapes as arguments to
/// [`is_sroa_unlockable_candidate`] helpers, and whose unlock pays for itself.
///
/// Cost model (all-or-nothing per aggregate root): the unlock clones every
/// escaping helper call, `cost = sum of their instruction counts`. It is
/// admitted only when `cost <= accesses`, the caller's loads and stores of
/// the root, each of which SROA turns into a register operation or nothing,
/// and `cost <= growth_allowance`, the caller's remaining inline budget. A
/// partial unlock is pure growth (the alloca stays in memory), so the test
/// is on the whole remaining escape set. It is monotone under the inliner's
/// own progress: cloning one helper lowers `cost` and the allowance by the
/// same amount and only adds accesses (the clone's own loads and stores now
/// name the root), so an unlock that starts also completes.
fn sroa_unlockable_aliases_of(
    func: &IrFunction,
    callee_map: &FxHashMap<String, CalleeData>,
    growth_allowance: usize,
) -> FxHashSet<Value> {
    if func.blocks.is_empty() {
        return FxHashSet::default();
    }
    // `param_alloca_values` is the block's own record of which allocas are
    // parameter homes, and it is exactly what `aggregate_sroa` skips. Deriving
    // the set from entry-block position instead (as this did) can both miss a
    // home (a local alloca emitted among the first `params.len()` instructions
    // would be treated as one) and invent one; the two passes must agree on
    // the definition or the inliner unlocks an alloca SROA then refuses.
    let param_homes: FxHashSet<Value> = func.param_alloca_values.iter().copied().collect();
    let mut candidate_roots: FxHashSet<Value> = FxHashSet::default();
    let mut root_size: FxHashMap<Value, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
                && *size > 0
                && *size <= 64
                && !param_homes.contains(dest)
            {
                candidate_roots.insert(*dest);
                root_size.insert(*dest, *size);
            }
        }
    }
    if candidate_roots.is_empty() {
        return FxHashSet::default();
    }
    let mut root_of: FxHashMap<Value, Value> = candidate_roots.iter().map(|&v| (v, v)).collect();
    // Aliases that address their root at offset 0 (copies and zero GEPs):
    // the only pointers a whole-object memset may name (form 4's rule 5b).
    let mut zero_offset: FxHashSet<Value> = candidate_roots.clone();
    // Monotone and bounded by the value count, so this terminates; the old
    // `0..4` bound silently stopped seeing aliases on a longer `gep`/`copy`
    // chain, which only ever *over*-reported escapes (a pessimisation, but a
    // silent one).
    loop {
        let mut added = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    }
                    | Instruction::GetElementPtr {
                        dest, base: src, ..
                    } => {
                        if let Some(&r) = root_of.get(src)
                            && root_of.insert(*dest, r).is_none()
                        {
                            added = true;
                            let zero_step = match inst {
                                Instruction::GetElementPtr { offset, .. } => {
                                    matches!(offset, Operand::Const(c) if c.to_i64() == Some(0))
                                }
                                _ => true,
                            };
                            if zero_step && zero_offset.contains(src) {
                                zero_offset.insert(*dest);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if !added {
            break;
        }
    }
    let mut disqualified: FxHashSet<Value> = FxHashSet::default();
    // Per root: memory accesses SROA would remove, and instructions the
    // unlock would clone.
    let mut accesses: FxHashMap<Value, usize> = FxHashMap::default();
    let mut unlock_cost: FxHashMap<Value, usize> = FxHashMap::default();
    // A memset's result must be unused for form 4 to delete the call. The
    // use set costs a pass over the function, so it is built only when a
    // memset-family call is present (this analysis runs every inline round).
    let mut used_values: FxHashSet<u32> = FxHashSet::default();
    let has_memset = func.blocks.iter().any(|b| {
        b.instructions.iter().any(
            |i| matches!(i, Instruction::Call { func, .. } if func == "memset" || func == "__memset_chk"),
        )
    });
    for block in func.blocks.iter().filter(|_| has_memset) {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                used_values.insert(v);
            });
        }
        block.terminator.for_each_used_value(|v| {
            used_values.insert(v);
        });
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { .. } | Instruction::Copy { .. } => {}
                Instruction::Load { ptr, .. } => {
                    if let Some(&r) = root_of.get(ptr) {
                        *accesses.entry(r).or_default() += 1;
                    }
                }
                Instruction::Store { val, ptr, .. } => {
                    if let Operand::Value(v) = val
                        && let Some(&r) = root_of.get(v)
                    {
                        disqualified.insert(r);
                    }
                    if let Some(&r) = root_of.get(ptr) {
                        *accesses.entry(r).or_default() += 1;
                    }
                }
                // A whole-object constant library memset is rewritten into
                // per-field stores by `aggregate_sroa` form 4 (rule 5b, same
                // `const_memset` predicate), so it neither escapes the root
                // nor costs an inline. Any other memset of a local object is
                // refused there and disqualifies the root here.
                Instruction::Call {
                    func: callee_name,
                    info,
                } if crate::passes::aggregate_sroa::const_memset(callee_name, info).is_some() => {
                    if let Some((p, _, len)) =
                        crate::passes::aggregate_sroa::const_memset(callee_name, info)
                        && let Some(&r) = root_of.get(&p)
                    {
                        let whole = zero_offset.contains(&p)
                            && root_size.get(&r).is_some_and(|&n| n as i64 == len)
                            && info.dest.is_none_or(|d| !used_values.contains(&d.0));
                        if !whole {
                            disqualified.insert(r);
                        }
                    }
                }
                Instruction::Call {
                    func: callee_name,
                    info,
                } => {
                    let helper = callee_map
                        .get(callee_name)
                        .filter(|data| is_sroa_unlockable_candidate(data));
                    for arg in &info.args {
                        if let Operand::Value(v) = arg
                            && let Some(&r) = root_of.get(v)
                        {
                            match helper {
                                Some(data) => {
                                    let size: usize =
                                        data.blocks.iter().map(|b| b.instructions.len()).sum();
                                    *unlock_cost.entry(r).or_default() += size;
                                }
                                None => {
                                    disqualified.insert(r);
                                }
                            }
                        }
                    }
                }
                _ => {
                    inst.for_each_used_value(|v_id| {
                        if let Some(&r) = root_of.get(&Value(v_id)) {
                            disqualified.insert(r);
                        }
                    });
                }
            }
        }
    }
    // A terminator that names the address (`return &local`, a branch on it)
    // escapes it just as an instruction would; this scan was missing.
    for block in &func.blocks {
        block.terminator.for_each_used_value(|v| {
            if let Some(&r) = root_of.get(&Value(v)) {
                disqualified.insert(r);
            }
        });
    }
    for (&root, &cost) in &unlock_cost {
        let paid = accesses.get(&root).copied().unwrap_or(0);
        if cost > paid || cost > growth_allowance {
            disqualified.insert(root);
        }
    }
    root_of
        .into_iter()
        .filter_map(|(v, r)| (!disqualified.contains(&r)).then_some(v))
        .collect()
}

/// Size/shape test for a declared-`inline` internal-linkage callee that
/// contains a loop and may be inlined at an in-loop site despite the
/// static-inline limit and the caller-size caps (`select_inline_site`).
/// GCC gives declared-inline bodies `max-inline-insns-single`; LCCC's
/// static-inline tier stops at MAX_INLINE_INSTRUCTIONS, so a short scanning
/// loop such as zstd's `ZSTD_count` (run once per match in every block
/// compressor) stayed a call: 163 call sites in libzstd versus 5 for GCC.
fn is_hot_loop_helper_candidate(data: &CalleeData) -> bool {
    let instructions: usize = data.blocks.iter().map(|b| b.instructions.len()).sum();
    data.is_static_inline
        && !data.is_always_inline
        && data.has_loops
        && !data.is_recursive
        && !data.const_trip_only
        && !data.const_branch_only
        && instructions <= MAX_HOT_LOOP_HELPER_INSTRUCTIONS
        && data.blocks.len() <= MAX_HOT_LOOP_HELPER_BLOCKS
}

/// Blocks of `lp` executed on every iteration that reaches a back edge: each
/// dominates every latch (in-loop predecessor of the header). A static, profile-
/// free "must execute per iteration" proof used for hot-site decisions.
fn every_iteration_blocks_of(
    cfg: &crate::ir::analysis::CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
) -> Vec<usize> {
    let latches: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p))
        .collect();
    if latches.is_empty() {
        return Vec::new();
    }
    lp.body
        .iter()
        .copied()
        .filter(|&b| {
            latches
                .iter()
                .all(|&latch| crate::ir::analysis::dominates_idom(&cfg.idom, b, latch))
        })
        .collect()
}

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
    hot_leaf_budget_remaining: Option<usize>,
    size_optimized: bool,
    loop_blocks: &FxHashSet<usize>,
    every_iteration_blocks: &FxHashSet<usize>,
    devirtualized_callees: &FxHashSet<String>,
    caller_has_loops: bool,
    hot_direct: bool,
    sroa_unlockable_aliases: &FxHashSet<Value>,
) -> Option<(InlineCallSite, usize, bool, bool)> {
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
        // Constant-trip-only bodies are restricted to qualifying sites in
        // both passes (see the second pass for the size_optimized rationale).
        let const_trip_admitted = callee_data.const_trip_only
            && !size_optimized
            && const_trip_site_arg(
                callee_data.loop_bound_params,
                &callee_data.param_struct_sizes,
                &site.args,
            );
        if callee_data.const_trip_only && !const_trip_admitted {
            continue;
        }
        // Constant-branch-specialized `static inline` callees are handled in
        // pass 2 so caller budgets and hard/absolute caps always apply.
        if callee_data.const_branch_only {
            continue;
        }
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
        if aggregate_frame_should_stay_outlined(callee_data, site.pgo_force, size_optimized)
            || template_wrapper_should_wait_for_specialization(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
            || multisite_loop_wrapper_should_stay_outlined(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
            || (!const_trip_admitted
                && nested_loop_multisite_static_should_stay_outlined(
                    callee_data,
                    callee_inst_count,
                    loop_blocks.contains(&site.block_idx),
                    caller_has_section,
                    site.pgo_force,
                    size_optimized,
                ))
            || repeated_small_loop_clone_should_stay_outlined(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
        {
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
            return Some((site.clone(), callee_inst_count, use_relaxed, false));
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
        // Constant-trip tier: a body admitted only by that tier is cloned only
        // at a site that passes a small constant for a loop-bound parameter.
        // The tier is disabled at -Os, where a loop-body clone is a size cost
        // with no measured benefit. The multi-site loop guard below is
        // exempted only for this admitted case; every other callee keeps it.
        let const_trip_admitted = callee_data.const_trip_only
            && !size_optimized
            && const_trip_site_arg(
                callee_data.loop_bound_params,
                &callee_data.param_struct_sizes,
                &site.args,
            );
        if callee_data.const_trip_only && !const_trip_admitted {
            continue;
        }
        let const_branch_admitted = callee_data.const_branch_only
            && !size_optimized
            && const_branch_site_arg(
                callee_data.const_branch_params,
                &callee_data.param_struct_sizes,
                &site.args,
            );
        if callee_data.const_branch_only && !const_branch_admitted {
            continue;
        }
        let callee_inst_count: usize = callee_data
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum();
        if aggregate_frame_should_stay_outlined(callee_data, site.pgo_force, size_optimized)
            || template_wrapper_should_wait_for_specialization(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
            || multisite_loop_wrapper_should_stay_outlined(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
            || (!const_trip_admitted
                && nested_loop_multisite_static_should_stay_outlined(
                    callee_data,
                    callee_inst_count,
                    loop_blocks.contains(&site.block_idx),
                    caller_has_section,
                    site.pgo_force,
                    size_optimized,
                ))
            || repeated_small_loop_clone_should_stay_outlined(
                callee_data,
                callee_inst_count,
                caller_has_section,
                site.pgo_force,
                size_optimized,
            )
        {
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
        // For recursive callers, skip non-tiny, non-always_inline callees
        // (tiny callees were handled in the first pass) unless the callee
        // cannot grow the frame that recursion multiplies.
        if caller_is_recursive
            && !callee_data.is_always_inline
            && !is_frame_neutral_leaf(callee_data, callee_inst_count)
        {
            continue;
        }
        // Devirtualized hot leaf. A function-pointer template parameter
        // (zstd's `matchFound`) is specialized to one small static target;
        // when that call executes on EVERY loop iteration (its block
        // dominates every latch), leaving it outlined defeats the intended
        // specialization and costs call/return plus argument traffic per
        // iteration. Such a site bypasses the caller-size caps, charged to
        // the ordinary budget and then to a relative budget equal to the
        // caller's entry size (GCC's large-function-growth = 100%), which
        // also bounds kernel stack frames.
        //
        // Direct calls (`hot_direct`) qualify only for a callee the
        // programmer declared `inline` (GCC's max-inline-insns-single
        // versus -auto distinction), and only below the absolute caller cap:
        // an earlier, unconditional extension cloned two small direct hash
        // helpers into zstd's 2.5k-instruction doubleFast loop, which slowed
        // binary-input level-3 compression by 21-28 % on the screening host
        // with identical Callgrind counts. That caller stays excluded; the
        // admitted case is a declared-inline leaf such as zstd's
        // FSE_encodeSymbol, called on every iteration of a mid-sized
        // sequence-encoding loop.
        //
        // A declared-inline callee with its own loop (a scan such as zstd's
        // `ZSTD_count`) qualifies at any in-loop site, not only an
        // every-iteration one: it runs once per match, and its call
        // overhead (argument setup, a callee-saved frame, the return) is
        // paid on each. Same knob, budget and absolute caller cap, so the
        // 2.5k-instruction doubleFast loop above stays excluded.
        // SROA-unlocking helper: the alias set already admits only an
        // aggregate whose whole escape set is paid for by the memory
        // accesses SROA removes and fits the remaining budget (see
        // `sroa_unlockable_aliases_of`), so the caller-size caps do not
        // apply. Those caps bound frame and code growth, and this clone
        // shrinks the frame by the aggregate's slot.
        let sroa_unlocking_leaf = hot_direct
            && is_sroa_unlockable_candidate(callee_data)
            && site
                .args
                .iter()
                .any(|arg| matches!(arg, Operand::Value(v) if sroa_unlockable_aliases.contains(v)));
        let hot_inline_leaf = (hot_leaf_budget_remaining.is_some()
            && !size_optimized
            && ((every_iteration_blocks.contains(&site.block_idx)
                && is_hot_leaf_candidate(callee_data)
                && (devirtualized_callees.contains(&site.callee_name)
                    || (hot_direct && callee_data.is_static_inline && !caller_at_absolute_cap)))
                || (hot_direct
                    && !caller_at_absolute_cap
                    && loop_blocks.contains(&site.block_idx)
                    && is_hot_loop_helper_candidate(callee_data))))
            || (!size_optimized && sroa_unlocking_leaf);
        // A hot-leaf candidate above the ordinary limit is offered by
        // `find_inline_call_sites` only for this decision; every other site
        // keeps the historical exceeds-normal-limits exclusion.
        if callee_data.exceeds_normal_limits
            && !caller_has_section
            && !callee_data.is_single_call_site_static
            && !hot_inline_leaf
        {
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
            let size_cap_exempt = callee_data.is_single_call_site_static
                && callee_data.size_inline_cost <= MAX_SINGLE_SITE_LOOP_NEST_EXEMPTION_COST;

            // A constant-trip clone (see MAX_CONST_TRIP_COUNT) is bounded by
            // the per-caller budget and the hard/absolute caps below. The soft
            // cap would otherwise veto the second clone in a caller, as in
            // expat's sip24_final, where the 4-round site follows a 2-round
            // clone and needs the caller to grow past 200 instructions.
            if caller_too_large
                && !callee_data.is_always_inline
                && !size_cap_exempt
                && !const_trip_admitted
                && !const_branch_admitted
                && !hot_inline_leaf
                && !(in_loop
                    && !callee_data.has_loops
                    && callee_inst_count <= MAX_INLINE_INSTRUCTIONS)
                && (!caller_has_section || callee_inst_count > MAX_SMALL_INLINE_INSTRUCTIONS)
            {
                continue;
            }
            // Absolute cap: stop normal inlining for extremely large callers.
            // always_inline callees MUST still be inlined (C semantic requirement).
            if caller_at_absolute_cap && !callee_data.is_always_inline && !hot_inline_leaf {
                continue;
            }
            // Hard cap: stop normal inlining to prevent kernel stack overflow.
            // always_inline callees are still inlined (C semantic requirement),
            // but are limited by the always_inline budget.
            if caller_at_hard_cap && !callee_data.is_always_inline && !hot_inline_leaf {
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
            if hot_inline_leaf && callee_inst_count <= hot_leaf_budget_remaining.unwrap_or(0) {
                return Some((site.clone(), callee_inst_count, use_relaxed, true));
            }
            continue;
        }
        return Some((site.clone(), callee_inst_count, use_relaxed, false));
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

/// Switches for the inliner's later extensions. Each defaults on and is
/// reversible from the environment for A/B measurement and miscompile triage
/// (project convention). `run_passes` resolves the `CCC_NO_INLINE_*` switches
/// once per translation unit and hands them over through [`set_features`]
/// (the pass pipeline does not read the environment per call; see
/// `check_env_test_hygiene.sh`); tests pass explicit values instead of
/// mutating the process environment, which parallel tests share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InlineFeatures {
    /// Resolve constant-folded indirect calls to direct calls first.
    pub(crate) devirt: bool,
    /// Replace a processed caller's snapshot with its processed body.
    pub(crate) snapshot_refresh: bool,
    /// Iterate ordinary and second-pass always_inline expansion jointly.
    pub(crate) joint_fixpoint: bool,
    /// Devirtualized per-iteration hot-leaf budget bypass.
    pub(crate) hot_leaf: bool,
    /// Size and clone a refreshed snapshot after scalar cleanup.
    pub(crate) snapshot_cleanup: bool,
    /// Clean a size-capped caller in place before giving up on it.
    pub(crate) caller_cleanup: bool,
    /// Extend the hot-leaf admission to direct calls of declared-inline
    /// leaves, and admit declared-inline loop helpers at in-loop sites
    /// (both below the absolute caller cap).
    pub(crate) hot_direct: bool,
}

impl InlineFeatures {
    pub(crate) const ALL: Self = Self {
        devirt: true,
        snapshot_refresh: true,
        joint_fixpoint: true,
        hot_leaf: true,
        snapshot_cleanup: true,
        caller_cleanup: true,
        hot_direct: true,
    };
}

// A `thread_local` (not a `LazyLock`): worker threads compile several
// translation units in one process and `run_passes` re-resolves per unit,
// the same shape as the affine-fold and byte-compare knobs.
thread_local! {
    static FEATURES: std::cell::Cell<InlineFeatures> =
        const { std::cell::Cell::new(InlineFeatures::ALL) };
}

/// Install this translation unit's inliner feature switches.
pub(crate) fn set_features(features: InlineFeatures) {
    FEATURES.with(|f| f.set(features));
}

// `-fPIC`/`-shared` semantic interposition: a default-visibility external
// definition may be replaced by another module's symbol at dynamic-link
// time, so callers in the defining TU must keep the call. Mirrors GCC's
// default `-fsemantic-interposition` at -fPIC (off at -fPIE/-fno-pic —
// the kernel and static-link builds, where GCC inlines freely).
thread_local! {
    static SEMANTIC_INTERPOSITION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Record this translation unit's interposition mode (resolved once per
/// unit by the driver, the same shape as the affine-fold knobs).
/// Diagnostic configuration for SSA parameter substitution, resolved once per
/// translation unit by `run_passes` (`set_ssa_param_config`).
///
/// `CCC_NO_SSA_PARAM=1` disables substitution; `CCC_SSA_PARAM_SKIP=a,b` names
/// callees to leave alone; `CCC_SSA_PARAM_LOG=1` traces each substitution.
#[derive(Clone, Default)]
pub(crate) struct SsaParamConfig {
    pub(crate) enabled: bool,
    pub(crate) skip: crate::common::fx_hash::FxHashSet<String>,
    pub(crate) log: bool,
}

thread_local! {
    static SSA_PARAM_CONFIG: std::cell::RefCell<SsaParamConfig> =
        std::cell::RefCell::new(SsaParamConfig {
            enabled: true,
            skip: crate::common::fx_hash::FxHashSet::default(),
            log: false,
        });
}

/// Install this translation unit's substitution configuration. Called once per
/// `run_passes` invocation so a previous compile on the same worker thread
/// cannot leak its diagnostics into the next one.
pub(crate) fn set_ssa_param_config(config: SsaParamConfig) {
    SSA_PARAM_CONFIG.with(|cell| *cell.borrow_mut() = config);
}

fn ssa_param_config() -> SsaParamConfig {
    SSA_PARAM_CONFIG.with(|cell| cell.borrow().clone())
}

pub(crate) fn set_semantic_interposition(interposable: bool) {
    SEMANTIC_INTERPOSITION.with(|f| f.set(interposable));
}

fn inline_run_impl(module: &mut IrModule, size_optimized: bool, always_inline_only: bool) -> usize {
    inline_run_with(
        module,
        size_optimized,
        always_inline_only,
        FEATURES.with(|f| f.get()),
    )
}

fn inline_run_with(
    module: &mut IrModule,
    size_optimized: bool,
    always_inline_only: bool,
    features: InlineFeatures,
) -> usize {
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
    let InlineFeatures {
        devirt: devirt_enabled,
        snapshot_refresh: refresh_enabled,
        joint_fixpoint: joint_fixpoint_enabled,
        hot_leaf: hot_leaf_enabled,
        snapshot_cleanup: cleanup_enabled,
        caller_cleanup: caller_cleanup_enabled,
        hot_direct: hot_direct_enabled,
    } = features;
    let devirtualized = if devirt_enabled {
        devirtualize_constant_indirect_calls(module)
    } else {
        FxHashSet::default()
    };
    if debug_inline && !devirtualized.is_empty() {
        eprintln!(
            "[INLINE] Devirtualized constant indirect calls (caller, callee): {:?}",
            devirtualized
        );
    }
    let callee_ctx = callee_context(module);
    let mut callee_map = build_callee_map_with(module, &callee_ctx);
    if always_inline_only {
        callee_map.retain(|_, data| data.is_always_inline);
    }

    // Functions on a cycle of the direct call graph. Phase 0 meters only
    // these: an always_inline callee off every cycle can expose only callees
    // strictly below it, so draining it terminates without a work counter.
    let recursive_callees = functions_on_call_cycles(module);

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

    // ── One-level direct self-recursion clone ───────────────────────────
    // Runs before the ordinary rounds so every downstream decision (callee
    // sizes, `has_inlined_calls`, the recursive-caller caps) sees the cloned
    // bodies. It is a one-shot transform, so it cannot interact with the
    // fixpoint below. `-Os` keeps the small body it has; the always-inline-only
    // invocation is a restricted mode and must not add ordinary inlining.
    if !size_optimized && !always_inline_only && self_recursion_clone_enabled() {
        let cloned = clone_self_recursion_sites(
            module,
            &callee_map,
            &skip_list,
            &mut global_max_block_id,
            debug_inline,
        );
        if cloned > 0 {
            // The cloned bodies are the callees of later rounds: replace the
            // pre-clone snapshots so sizes and flags describe what is there.
            callee_map = build_callee_map_with(module, &callee_ctx);
            total_inlined += cloned;
        }
    }

    // Process each function as a potential caller
    for func_idx in 0..module.functions.len() {
        let inlined_before = total_inlined;
        let devirtualized_callees: FxHashSet<String> = devirtualized
            .iter()
            .filter(|(caller, _)| *caller == module.functions[func_idx].name)
            .map(|(_, callee)| callee.clone())
            .collect();
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
        // Relative growth allowance for hot in-loop declared-inline leaves
        // only (see `select_inline_site`): at most the caller's entry size.
        let mut hot_leaf_budget_remaining = hot_leaf_enabled.then(|| {
            MAX_INLINE_BUDGET_PER_CALLER.max(
                module.functions[func_idx]
                    .blocks
                    .iter()
                    .map(|b| b.instructions.len())
                    .sum(),
            )
        });
        let mut always_inline_budget_remaining = MAX_ALWAYS_INLINE_BUDGET_PER_CALLER;
        let mut pgo_force_budget_remaining = MAX_PGO_FORCE_INLINE_BUDGET_PER_CALLER;
        // During one -Os inliner invocation, clone a large ordinary callee at
        // most once into a given caller. This permits one profitable
        // specialization while avoiding repeated medium-body expansion.
        let mut size_inlined_large_callees: FxHashSet<String> = FxHashSet::default();
        // ── Phase 0: mandatory always_inline drain ───────────────────────
        // `__always_inline` is GCC's FORCE contract, not a scheduling
        // preference: the call must never survive (`extern __gnu_inline__
        // __always_inline__` wrappers — the kernel's whole fortify-string
        // family — have no out-of-line body, so one surviving call is a
        // hard undefined-symbol link error).  The contract cannot be left
        // to the economics below: the ordinary fixpoint inlines ONE site
        // per round under a 200-round cap, and always_inline callees that
        // replicate through macro dispatchers (each inlined clone
        // re-entering the same dispatcher) burn rounds faster than the
        // queue drains — integrity_audit.o's single fortify_memset_chk site
        // was starved exactly so and the vmlinux link died on the undefined
        // extern.
        //
        // Three laws make the phase sound:
        //   * priority: sites present at phase entry — every block whose
        //     label is <= the module-wide label maximum snapshot, since
        //     cloned blocks always take labels strictly above
        //     `global_max_block_id` — are drained before any clone-born
        //     site, so the contract never queues behind churn;
        //   * termination: only callees on a cycle of the direct call graph
        //     (`functions_on_call_cycles`) spend the bounded work counter
        //     (2x entry sites + 16), so a recursive dispatcher chain cannot
        //     chase the phase forever. Acyclic expansion is unmetered: every
        //     drained body exposes only callees strictly below it in the
        //     call DAG, so it terminates on its own -- a flat counter starved
        //     nested FORCE_INLINE templates (zstd's HUF body: 10 body_loop
        //     sites, each exposing encodeSymbol/addBits/flushBits) and left
        //     template calls out of line. `PHASE0_CALLER_GUARD_INSTRUCTIONS`
        //     bounds pathological DAG fan-out (diamond chains);
        //   * no budget gate: a mandatory inline is not an allocation
        //     decision, and phase successes do not touch the economics
        //     budgets the ordinary passes then spend purely on profit.
        let entry_max_label = global_max_block_id;
        let entry_always_inline_sites = find_inline_call_sites(
            &module.functions[func_idx],
            &callee_map,
            &skip_list,
            caller_has_section,
        )
        .into_iter()
        .filter(|s| {
            callee_map
                .get(&s.callee_name)
                .is_some_and(|d| d.is_always_inline)
        })
        .count();
        let mut phase0_budget = 2 * entry_always_inline_sites + 16;
        let mut phase0_caller_size: usize = module.functions[func_idx]
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum();
        while phase0_budget > 0 && phase0_caller_size <= PHASE0_CALLER_GUARD_INSTRUCTIONS {
            let sites = find_inline_call_sites(
                &module.functions[func_idx],
                &callee_map,
                &skip_list,
                caller_has_section,
            );
            let is_mandatory = |s: &InlineCallSite| {
                callee_map
                    .get(&s.callee_name)
                    .is_some_and(|d| d.is_always_inline)
            };
            // Original-label sites first; clone-born sites only once the
            // last original is gone.  Splits preserve the continuation's
            // label (see inline_call_site), so every surviving entry site
            // stays entry-class for the whole drain.  Among clone-born
            // sites, lowest program order first: churn from an inlined
            // dispatcher always splices at positions strictly AFTER the
            // pre-call remainder of the drained block, so position order
            // drains those leftovers before any churn can overtake them.
            let site = sites
                .iter()
                .find(|s| {
                    is_mandatory(s)
                        && module.functions[func_idx].blocks[s.block_idx].label.0 <= entry_max_label
                })
                .or_else(|| {
                    sites
                        .iter()
                        .filter(|s| is_mandatory(s))
                        .min_by_key(|s| (s.block_idx, s.inst_idx))
                })
                .cloned();
            let Some(site) = site else { break };
            let callee_data = &callee_map[&site.callee_name];
            if !inline_call_site(
                &mut module.functions[func_idx],
                &site,
                callee_data,
                &mut global_max_block_id,
            ) {
                break;
            }
            phase0_caller_size += callee_data
                .blocks
                .iter()
                .map(|b| b.instructions.len())
                .sum::<usize>();
            if recursive_callees.contains(&site.callee_name) {
                phase0_budget -= 1;
            }
            total_inlined += 1;
            module.functions[func_idx].has_inlined_calls = true;
            if debug_inline {
                eprintln!(
                    "[INLINE] Phase-0 drained always_inline '{}' in '{}'",
                    site.callee_name, module.functions[func_idx].name
                );
            }
        }

        // Iterate to handle chains of inlined calls (A calls B calls C, all small inline).
        // Limit iterations to prevent infinite loops from recursive inline functions.
        // Ordinary and second-pass always_inline expansion form one bounded
        // fixpoint. A mandatory raw template can expose calls the ordinary
        // policy always accepts (tiny/static-inline). Previously those calls
        // survived because ordinary rounds had already finished. The rounds
        // and second-pass budget are shared, so no new growth bound appears.
        let mut ordinary_rounds_left: usize = 200;
        let mut second_pass_budget = MAX_ALWAYS_INLINE_SECOND_PASS_BUDGET;
        let mut second_pass_rounds_left = MAX_ALWAYS_INLINE_SECOND_PASS_ROUNDS;
        // Size of this caller right after its last in-place cleanup; see the
        // capped-caller re-measurement below.
        let mut size_after_cleanup: Option<usize> = None;
        loop {
            let mut second_pass_inlined = false;
            // T5 (round_cap): a budget exit counts only if the last round still
            // inlined something; this flag records exactly that.
            let mut ordinary_round_inlined = false;
            while ordinary_rounds_left > 0 {
                ordinary_rounds_left -= 1;
                ordinary_round_inlined = false;
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
                let caller_at_absolute_cap =
                    caller_inst_count > MAX_CALLER_INSTRUCTIONS_ABSOLUTE_CAP;

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
                                        let dummy = crate::ir::reexports::IrFunction {
                                            pass_policy: Default::default(),
                                            name: callee_name.clone(),
                                            return_type: crate::common::types::IrType::I32,
                                            params: vec![],
                                            blocks: callee.blocks.clone(),
                                            is_variadic: false,
                                            is_declaration: false,
                                            is_static: false,
                                            is_inline: false,
                                            is_always_inline: false,
                                            x86_isa_needs: Default::default(),
                                            is_noinline: false,
                                            target_features: Vec::new(),
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
                                            calling_convention:
                                                crate::common::types::CallingConvention::Default,
                                            is_fastcall: false,
                                            regparm: None,
                                            is_naked: false,
                                            no_instrument: false,
                                            global_init_label_blocks: vec![],
                                            ret_eightbyte_classes: vec![],
                                            ret_is_f128_sse: false,
                                            is_gnu_inline_def: false,
                                            loop_promoted_f64_values: Vec::new(),
                                            ssp: Default::default(),
                                            body_span: crate::common::source::Span::dummy(),
                                            terminator_spans: Default::default(),
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
                    call_sites
                        .retain(|site| !size_inlined_large_callees.contains(&site.callee_name));
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
                let (loop_blocks, every_iteration_blocks): (FxHashSet<usize>, FxHashSet<usize>) =
                    if needs_loop_info {
                        let cfg =
                            crate::ir::analysis::CfgAnalysis::build(&module.functions[func_idx]);
                        let loops = loop_analysis::find_natural_loops(
                            cfg.num_blocks,
                            &cfg.preds,
                            &cfg.succs,
                            &cfg.idom,
                        );
                        let mut s = FxHashSet::default();
                        let mut every = FxHashSet::default();
                        for lp in &loops {
                            s.extend(lp.body.iter().copied());
                            every.extend(every_iteration_blocks_of(&cfg, lp));
                        }
                        (s, every)
                    } else {
                        (FxHashSet::default(), FxHashSet::default())
                    };
                let caller_has_loops = !loop_blocks.is_empty();
                let sroa_unlockable_aliases = if hot_direct_enabled && !size_optimized {
                    sroa_unlockable_aliases_of(
                        &module.functions[func_idx],
                        &callee_map,
                        budget_remaining.max(hot_leaf_budget_remaining.unwrap_or(0)),
                    )
                } else {
                    FxHashSet::default()
                };
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
                    hot_leaf_budget_remaining,
                    size_optimized,
                    &loop_blocks,
                    &every_iteration_blocks,
                    &devirtualized_callees,
                    caller_has_loops,
                    hot_direct_enabled,
                    &sroa_unlockable_aliases,
                );
                let (site, callee_inst_count, _use_relaxed, hot_leaf_charge) = match found_site {
                    Some(s) => s,
                    None => {
                        // The size caps measure the caller as pasted: every
                        // inlined clone still carries its parameter homes,
                        // copies and dead residue until the post-inline
                        // cleanup (zstd's ZSTD_encodeSequences_body: 885
                        // here, 450 after this cleanup), so a capped caller
                        // refuses calls its real size admits. Before giving
                        // up, run the function-local cleanup in place and
                        // select again against the size the caller really
                        // has. It reruns only after the caller grew past its
                        // last cleaned size, so every cleanup is paid for by
                        // newly inlined work, and the shared round counter
                        // still bounds the loop.
                        if caller_cleanup_enabled
                            && caller_too_large
                            && size_after_cleanup.is_none_or(|n| caller_inst_count > n)
                            && !scalar_cleanup_blocked(&module.functions[func_idx], module)
                        {
                            scalar_cleanup(&mut module.functions[func_idx]);
                            let cleaned: usize = module.functions[func_idx]
                                .blocks
                                .iter()
                                .map(|b| b.instructions.len())
                                .sum();
                            size_after_cleanup = Some(cleaned);
                            if debug_inline {
                                eprintln!(
                                    "[INLINE] Cleaned capped caller '{}': {} -> {} instructions",
                                    module.functions[func_idx].name, caller_inst_count, cleaned
                                );
                            }
                            if cleaned < caller_inst_count {
                                continue;
                            }
                        }
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
                    } else if hot_leaf_charge {
                        hot_leaf_budget_remaining =
                            hot_leaf_budget_remaining.map(|b| b.saturating_sub(callee_inst_count));
                    } else {
                        budget_remaining = budget_remaining.saturating_sub(callee_inst_count);
                    }
                    total_inlined += 1;
                    ordinary_round_inlined = true;
                    module.functions[func_idx].has_inlined_calls = true;
                } else {
                    break;
                }
            }
            if ordinary_rounds_left == 0 && ordinary_round_inlined {
                round_cap::note_cap_hit(round_cap::Site::InlineOrdinaryRounds);
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
            let mut second_pass_round_inlined = false;
            while second_pass_rounds_left > 0 {
                second_pass_rounds_left -= 1;
                second_pass_round_inlined = false;
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
                                        let dummy = crate::ir::reexports::IrFunction {
                                            pass_policy: Default::default(),
                                            name: callee_name.clone(),
                                            return_type: crate::common::types::IrType::I32,
                                            params: vec![],
                                            blocks: callee.blocks.clone(),
                                            is_variadic: false,
                                            is_declaration: false,
                                            is_static: false,
                                            is_inline: false,
                                            is_always_inline: false,
                                            x86_isa_needs: Default::default(),
                                            is_noinline: false,
                                            target_features: Vec::new(),
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
                                            calling_convention:
                                                crate::common::types::CallingConvention::Default,
                                            is_fastcall: false,
                                            regparm: None,
                                            is_naked: false,
                                            no_instrument: false,
                                            global_init_label_blocks: vec![],
                                            ret_eightbyte_classes: vec![],
                                            ret_is_f128_sse: false,
                                            is_gnu_inline_def: false,
                                            loop_promoted_f64_values: Vec::new(),
                                            ssp: Default::default(),
                                            body_span: crate::common::source::Span::dummy(),
                                            terminator_spans: Default::default(),
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
                            validate_function_values(
                                &module.functions[func_idx],
                                &site.callee_name,
                            );
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
                        second_pass_inlined = true;
                        second_pass_round_inlined = true;
                        module.functions[func_idx].has_inlined_calls = true;
                        if !is_tiny && !is_small && !caller_has_section {
                            second_pass_budget =
                                second_pass_budget.saturating_sub(callee_inst_count);
                        }
                        found = true;
                        break; // Re-scan after each inline
                    }
                }
                if !found {
                    break;
                }
            }
            if second_pass_rounds_left == 0 && second_pass_round_inlined {
                round_cap::note_cap_hit(round_cap::Site::InlineSecondPassRounds);
            }
            if !joint_fixpoint_enabled || !second_pass_inlined || ordinary_rounds_left == 0 {
                break;
            }
        }

        // Bottom-up snapshot refresh. A later caller should clone this
        // function's processed body, not the raw pre-inlining snapshot: the
        // raw body re-exposes calls already expanded here, after that later
        // caller may have exhausted its size budget (zstd's encode loop kept
        // ten outlined bit-writer calls this way). Refresh only when the
        // processed body passes the identical eligibility rules, judged
        // against the same invocation-entry call graph, and cloning it cannot
        // duplicate grown work (see `refresh_callee_snapshot`). Otherwise
        // retain the old snapshot, preserving gnu_inline, always_inline and
        // section-sensitive wrapper behavior exactly.
        if refresh_enabled && total_inlined != inlined_before {
            refresh_callee_snapshot(
                module,
                func_idx,
                &callee_ctx,
                &mut callee_map,
                cleanup_enabled,
            );
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
                    Instruction::Call {
                        func: f,
                        info: _info,
                    } => {
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
    /// The callee's normalized `target("...")` feature set; see
    /// [`target_isa_allows_inline`].
    target_features: Vec<String>,
    /// What the body needs from the x86 ISA beyond `target_features`.
    x86_isa_needs: crate::ir::module::X86IsaNeeds,
    /// The body computes with floating-point or SIMD values.
    uses_fp_or_simd: bool,
    /// Builtin assumptions under which this body was formed. Cloning must
    /// conservatively merge these into the caller before later synthesis.
    builtin_policy: crate::common::pass_policy::BuiltinPolicy,
    ssp: crate::ir::module::SspInfo,
    /// For each param, Some(size) if it's a struct-by-value parameter, None otherwise.
    param_struct_sizes: Vec<Option<usize>>,
    /// `IrFunction::param_alloca_values`: lowering's record of each
    /// parameter's home alloca (see [`inlined_param_homes`]).
    param_alloca_values: Vec<Value>,
    /// Per parameter, the home lowering creates: `(ty, size, align)`.
    param_home_shapes: Vec<(IrType, usize, usize)>,
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
    /// The whole module contains exactly one call to this callee. Inlining is
    /// then size-neutral (the outlined body disappears), so size-based vetoes
    /// must not fire. Independent of `is_single_call_site_static`, which
    /// additionally requires an ordinary `static` definition.
    single_call_site: bool,
    /// Stack bytes the callee's body forces on *every* inline site: the sum of
    /// its non-promotable alloca sizes (see `AGG_FRAME_SLOT_BYTES`).
    /// Instruction-count costs are blind to this, which is how a ~15-instruction
    /// helper carrying a 44-byte `struct biosregs` still looked "small".
    agg_frame_bytes: usize,
    /// Module-wide direct calls (including a redirected asm label) observed
    /// before inlining.  Multi-site wrappers duplicate their descendants.
    direct_call_count: usize,
    /// Parameters that bound a loop exit (see `loop_bound_param_mask`).
    loop_bound_params: u64,
    /// Admitted only by the constant-trip tier: the body fails the general
    /// static-loop limits, so ordinary sites must not clone it. Such a callee
    /// is selected only at a call site with a qualifying constant argument.
    const_trip_only: bool,
    /// Parameters tested in a `Switch` or `CondBranch` compare against a
    /// compile-time constant (see `const_branch_param_mask`).
    const_branch_params: u64,
    /// Acyclic leaf `static inline` body exceeding `MAX_INLINE_INSTRUCTIONS` /
    /// `MAX_INLINE_BLOCKS_NO_LOOPS`, admitted only at call sites passing a
    /// compile-time constant for a branch-controlling parameter (e.g.
    /// `is_invalid_utf8(p, 2|3|4)` in libexpat's `normal_contentTok`).
    const_branch_only: bool,
    /// Whether recursively expanding ordinary eligible callees reaches a loop.
    /// A tiny wrapper can otherwise conceal a large loop nest from the raw
    /// tiny/small threshold used by the fixed-point inliner.
    has_inlineable_loop_descendant: bool,
    /// A prior inliner invocation has already expanded a descendant into this
    /// body. This persists across pipeline invocations, unlike a fresh raw
    /// call-graph snapshot, and identifies wrappers that have acquired a loop
    /// body after their initial tiny/small eligibility decision.
    has_inlined_calls: bool,
    /// Whether this callee is a `static` function (including `static inline`)
    /// with exactly one call site in the whole module. Such callees are dead
    /// after inlining (net code shrink), so they are exempt from the
    /// caller-size caps and the per-caller inlining budget, matching GCC's
    /// -O2 behavior (`-finline-functions-called-once` applies regardless of
    /// the `inline` keyword: a single-site body that is never address-taken
    /// disappears whether or not it was declared `inline`).
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

/// Bit mask of parameters (index < 64) that bound the exit test of a natural
/// loop: the parameter's entry value, through plain copies, is an operand of
/// the compare feeding a loop-exiting conditional branch. A parameter that is
/// reassigned inside the loop reaches the compare through a phi, so it is not
/// counted and the tier cannot admit it.
fn collect_unmutated_param_values(
    func: &IrFunction,
) -> (FxHashMap<Value, usize>, FxHashMap<Value, Value>) {
    let mut param_of: FxHashMap<Value, usize> = FxHashMap::default();
    let mut copy_src: FxHashMap<Value, Value> = FxHashMap::default();
    let mut entry_allocas: FxHashSet<u32> = FxHashSet::default();
    if let Some(entry) = func.blocks.first() {
        for inst in &entry.instructions {
            if let Instruction::Alloca {
                dest,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                entry_allocas.insert(dest.0);
            }
        }
    }
    let mut alloca_store_param: FxHashMap<u32, Option<usize>> = FxHashMap::default();
    let mut raw_param_ref: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            {
                param_of.insert(*dest, *param_idx);
                raw_param_ref.insert(dest.0, *param_idx);
            }
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Store {
                    ptr,
                    val: Operand::Value(v),
                    volatile: false,
                    ..
                } if entry_allocas.contains(&ptr.0) => {
                    if let Some(&p) = raw_param_ref.get(&v.0) {
                        alloca_store_param
                            .entry(ptr.0)
                            .and_modify(|e| *e = None)
                            .or_insert(Some(p));
                    } else {
                        alloca_store_param.insert(ptr.0, None);
                    }
                    if entry_allocas.contains(&v.0) {
                        alloca_store_param.insert(v.0, None);
                    }
                }
                Instruction::Load {
                    ptr,
                    volatile: false,
                    ..
                } if entry_allocas.contains(&ptr.0) => {}
                Instruction::Alloca { .. } => {}
                _ => {
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op
                            && entry_allocas.contains(&v.0)
                        {
                            alloca_store_param.insert(v.0, None);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        if entry_allocas.contains(&v.0) {
                            alloca_store_param.insert(v.0, None);
                        }
                    });
                }
            }
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load {
                    dest,
                    ptr,
                    volatile: false,
                    ..
                } => {
                    if let Some(Some(p)) = alloca_store_param.get(&ptr.0) {
                        param_of.insert(*dest, *p);
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                }
                | Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    ..
                } => {
                    copy_src.insert(*dest, *src);
                }
                _ => {}
            }
        }
    }
    (param_of, copy_src)
}

fn loop_bound_param_mask(func: &IrFunction) -> u64 {
    if func.blocks.is_empty() {
        return 0;
    }
    let mut param_of: FxHashMap<Value, usize> = FxHashMap::default();
    let mut copy_src: FxHashMap<Value, Value> = FxHashMap::default();
    let mut param_const_adj: FxHashMap<Value, usize> = FxHashMap::default();
    let mut cmp_operands: FxHashMap<Value, (Operand, Operand)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::ParamRef {
                    dest, param_idx, ..
                } => {
                    param_of.insert(*dest, *param_idx);
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => {
                    copy_src.insert(*dest, *src);
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add | IrBinOp::Sub,
                    lhs: Operand::Value(v),
                    rhs: Operand::Const(_),
                    ..
                } => {
                    if let Some(&p) = param_of.get(v) {
                        param_const_adj.insert(*dest, p);
                    }
                }
                Instruction::Cmp { dest, lhs, rhs, .. } => {
                    cmp_operands.insert(*dest, (*lhs, *rhs));
                }
                _ => {}
            }
        }
    }
    // Second sweep for param ± const across mem2reg Copies (`v = Copy(param); i = Sub(v, 1)`).
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add | IrBinOp::Sub,
                lhs: Operand::Value(v),
                rhs: Operand::Const(_),
                ..
            } = inst
            {
                let mut r = *v;
                for _ in 0..8 {
                    match copy_src.get(&r) {
                        Some(&s) => r = s,
                        None => break,
                    }
                }
                if let Some(&p) = param_of.get(&r) {
                    param_const_adj.insert(*dest, p);
                }
            }
        }
    }
    // Follow copy chains to their root, bounded so a cyclic copy (not expected
    // in SSA, but cheap to defend against) cannot loop forever.
    let root = |mut v: Value| -> Value {
        for _ in 0..8 {
            match copy_src.get(&v) {
                Some(&s) => v = s,
                None => break,
            }
        }
        v
    };
    let param_of_operand = |op: &Operand| -> Option<usize> {
        match op {
            Operand::Value(v) => param_of.get(&root(*v)).copied(),
            Operand::Const(_) => None,
        }
    };

    let label_index: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let loops =
        loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);

    let mut mask = 0u64;
    for lp in &loops {
        for &bi in &lp.body {
            let Terminator::CondBranch {
                cond: Operand::Value(cond),
                true_label,
                false_label,
            } = &func.blocks[bi].terminator
            else {
                continue;
            };
            let exits = [true_label, false_label]
                .iter()
                .any(|l| label_index.get(l).is_some_and(|&j| !lp.body.contains(&j)));
            if !exits {
                continue;
            }
            let Some((lhs, rhs)) = cmp_operands.get(&root(*cond)) else {
                continue;
            };
            for idx in [param_of_operand(lhs), param_of_operand(rhs)]
                .into_iter()
                .flatten()
            {
                if idx < 64 {
                    mask |= 1u64 << idx;
                }
            }
            // Countdown helper shape (`for (int i = n - 1; i >= 0; i--)`):
            // header Phi initialized from `param` or `param ± const` outside
            // the loop and compared against a constant exit threshold.
            let phi_cand = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(_)) | (Operand::Const(_), Operand::Value(v)) => {
                    Some(root(*v))
                }
                _ => None,
            };
            if let Some(pv) = phi_cand {
                for h_inst in &func.blocks[lp.header].instructions {
                    let Instruction::Phi { dest, incoming, .. } = h_inst else {
                        break;
                    };
                    if *dest != pv {
                        continue;
                    }
                    for (in_op, in_blk) in incoming {
                        let Some(&pred_idx) = label_index.get(in_blk) else {
                            continue;
                        };
                        if lp.body.contains(&pred_idx) {
                            continue;
                        }
                        if let Some(idx) = param_of_operand(in_op).or_else(|| match in_op {
                            Operand::Value(iv) => param_const_adj.get(&root(*iv)).copied(),
                            Operand::Const(_) => None,
                        }) {
                            if idx < 64 {
                                mask |= 1u64 << idx;
                            }
                        }
                    }
                }
            }
        }
    }
    mask
}

/// True when a call passes a small compile-time constant for a parameter that
/// bounds a loop exit of `callee`. Such a site is a constant-trip clone
/// candidate; the constant bounds the loop regardless of the other arguments.
/// Struct-by-value parameters are never scalar constants and are excluded.
fn const_trip_site_arg(
    loop_bound_params: u64,
    param_struct_sizes: &[Option<usize>],
    args: &[Operand],
) -> bool {
    if loop_bound_params == 0 {
        return false;
    }
    args.iter().take(64).enumerate().any(|(i, arg)| {
        loop_bound_params & (1u64 << i) != 0
            && param_struct_sizes.get(i).is_some_and(Option::is_none)
            && match arg {
                Operand::Const(c) => c
                    .to_i64()
                    .is_some_and(|v| (MIN_CONST_TRIP_COUNT..=MAX_CONST_TRIP_COUNT).contains(&v)),
                Operand::Value(_) => false,
            }
    })
}

/// Instruction and block ceilings for acyclic leaf `static inline` helpers
/// admitted only at call sites passing a compile-time constant for a
/// branch-controlling parameter (`Switch` selector or `CondBranch` compare
/// against a constant). Once specialized, SCCP/CFG simplification folds the
/// selector dispatch away so only the active arm remains in the caller.
const MAX_CONST_BRANCH_INLINE_INSTRUCTIONS: usize = 128;
const MAX_CONST_BRANCH_INLINE_BLOCKS: usize = 32;

fn const_branch_param_mask(func: &IrFunction) -> u64 {
    if func.blocks.is_empty() {
        return 0;
    }
    let (param_of, copy_src) = collect_unmutated_param_values(func);
    let root = |mut v: Value| -> Value {
        for _ in 0..8 {
            match copy_src.get(&v) {
                Some(&s) => v = s,
                None => break,
            }
        }
        v
    };
    let param_of_op = |op: &Operand| -> Option<usize> {
        match op {
            Operand::Value(v) => param_of.get(&root(*v)).copied(),
            Operand::Const(_) => None,
        }
    };
    let mut cmp_operands: FxHashMap<Value, (Operand, Operand)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Cmp { dest, lhs, rhs, .. } = inst {
                cmp_operands.insert(*dest, (*lhs, *rhs));
            }
        }
    }
    let mut mask = 0u64;
    for block in &func.blocks {
        match &block.terminator {
            Terminator::Switch { val, .. } => {
                if let Some(idx) = param_of_op(val)
                    && idx < 64
                {
                    mask |= 1u64 << idx;
                }
            }
            Terminator::CondBranch { cond, .. } => {
                if let Some(idx) = param_of_op(cond)
                    && idx < 64
                {
                    mask |= 1u64 << idx;
                }
                if let Operand::Value(cv) = cond
                    && let Some((lhs, rhs)) = cmp_operands.get(&root(*cv))
                {
                    match (lhs, rhs) {
                        (opnd, Operand::Const(_)) | (Operand::Const(_), opnd) => {
                            if let Some(idx) = param_of_op(opnd)
                                && idx < 64
                            {
                                mask |= 1u64 << idx;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    mask
}

fn const_branch_site_arg(
    const_branch_params: u64,
    param_struct_sizes: &[Option<usize>],
    args: &[Operand],
) -> bool {
    if const_branch_params == 0 {
        return false;
    }
    args.iter().take(64).enumerate().any(|(i, arg)| {
        const_branch_params & (1u64 << i) != 0
            && param_struct_sizes.get(i).is_some_and(Option::is_none)
            && matches!(arg, Operand::Const(_))
    })
}

/// The inlined callee's parameter homes, in parameter order, as
/// `(home, ty, size)` in the caller's value space (`value_offset` applied).
///
/// Lowering records every parameter's home alloca in
/// `IrFunction::param_alloca_values`, one per IR parameter (sret included),
/// and the backend treats that record as authoritative; homes are resolved
/// by value id against it. A home optimization deleted -- dead, because the
/// parameter lives on as SSA `ParamRef` values -- is `None`. Identifying
/// homes positionally as "the first N allocas of the entry block" is wrong
/// once a home is gone: after nested inlining the next alloca is the inner
/// callee's temporary, and the argument bridge then stores through that
/// slot, with its type, ahead of the slot's own definition (invalid IR; the
/// ParamRef-store filter would also treat the temporary as a home).
///
/// Without a record (IR not built by lowering, or a record a rewrite
/// cleared), live ParamRefs get fresh ABI-shaped homes. No unrelated local
/// is guessed to be a parameter home. `None` overall means a recorded home
/// sits outside the entry block, where the entry-block bridge could not
/// dominate it; the caller refuses to inline.
fn inlined_param_homes(
    callee: &CalleeData,
    inlined_blocks: &[BasicBlock],
    value_offset: u32,
) -> Option<Vec<Option<(Value, IrType, usize)>>> {
    let alloca_of = |inst: &Instruction| match inst {
        Instruction::Alloca { dest, ty, size, .. } => Some((*dest, *ty, *size)),
        _ => None,
    };
    let entry = inlined_blocks.first()?;
    if callee.param_alloca_values.len() != callee.num_params {
        // No provenance is not evidence that a leading local is a parameter.
        // Live ParamRefs receive fresh homes below; dead parameters need none.
        return Some(vec![None; callee.num_params]);
    }
    let mut homes = Vec::with_capacity(callee.num_params);
    for (index, recorded) in callee.param_alloca_values.iter().enumerate() {
        let want = recorded.0.checked_add(value_offset)?;
        let find = |block: &BasicBlock| {
            block
                .instructions
                .iter()
                .filter_map(alloca_of)
                .find(|(dest, _, _)| dest.0 == want)
        };
        match find(entry) {
            Some(home) => {
                // ABI expansion can associate a source vector object's Ptr
                // alloca with a scalar piece (e.g. U64 on i686). Identity is
                // not enough: a bridge through that object would truncate or
                // overwrite it. Refuse until lowering supplies piecewise homes.
                if callee
                    .param_home_shapes
                    .get(index)
                    .is_some_and(|&(ty, size, _)| ty != home.1 || size > home.2)
                {
                    return None;
                }
                homes.push(Some(home));
            }
            None if inlined_blocks[1..].iter().any(|b| find(b).is_some()) => return None,
            None => homes.push(None),
        }
    }
    Some(homes)
}

/// The home alloca lowering creates for `param` (`func_lowering`'s parameter
/// prologue): `(ty, size, align)`.
fn param_home_shape(param: &crate::ir::module::IrParam) -> (IrType, usize, usize) {
    let size = param
        .ty
        .size()
        .max(param.struct_size.unwrap_or(param.ty.size()));
    let align = param
        .struct_align
        .into_iter()
        .chain(param.param_align)
        .max()
        .unwrap_or(0);
    (param.ty, size, align)
}

mod devirt;
mod self_recursion;
use devirt::devirtualize_constant_indirect_calls;
pub(crate) use self_recursion::inline_self_recursion;

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
    for (_, resolver, _, _) in &module.ifuncs {
        names.insert(resolver.clone());
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

/// Module-wide facts shared by every callee snapshot in one inliner invocation.
/// A refreshed snapshot deliberately reuses the invocation-entry call graph:
/// eligibility is then judged exactly as it was for the raw snapshot.
struct CalleeContext {
    call_counts: FxHashMap<String, usize>,
    value_referenced: FxHashSet<String>,
    asm_templates: Vec<String>,
    call_site_counts: FxHashMap<String, usize>,
    /// For callees with exactly one entry call site: the function holding it.
    sole_caller: FxHashMap<String, String>,
    debug_callee: bool,
    /// This unit was built `-fPIC`/`-shared`: default-visibility external
    /// definitions are semantically interposable, so callers keep the call.
    semantic_interposition: bool,
}

fn callee_context(module: &IrModule) -> CalleeContext {
    let call_counts = direct_call_counts(module);
    let (value_referenced, asm_templates) = collect_value_referenced_functions(module);
    let mut call_site_counts: FxHashMap<String, usize> = FxHashMap::default();
    let mut sole_caller: FxHashMap<String, String> = FxHashMap::default();
    for f in &module.functions {
        for block in &f.blocks {
            for inst in &block.instructions {
                if let Instruction::Call {
                    func: callee_name, ..
                } = inst
                {
                    *call_site_counts.entry(callee_name.clone()).or_insert(0) += 1;
                    sole_caller.insert(callee_name.clone(), f.name.clone());
                }
            }
        }
    }
    sole_caller.retain(|callee, _| call_site_counts.get(callee) == Some(&1));
    CalleeContext {
        call_counts,
        value_referenced,
        asm_templates,
        call_site_counts,
        sole_caller,
        debug_callee: std::env::var("CCC_INLINE_DEBUG").is_ok(),
        semantic_interposition: SEMANTIC_INTERPOSITION.with(|f| f.get()),
    }
}

/// GCC at -O2 inlines small external-linkage DEFINITIONS into callers of the
/// same translation unit while keeping the out-of-line copy: the body is
/// this TU's own code and — outside -fPIC — cannot be interposed out from
/// under a direct call (glibc's `ffs`, zlib's helpers, every non-static
/// wrapper). Three gates keep admission on the proven class:
///
/// * `-fPIC`/`-shared` semantic interposition refuses external admission
///   entirely (a dynamic symbol may be replaced by another module's).
/// * Scalar returns only: the inliner's return-value merge is vetted for
///   scalars; two-eightbyte [Sse, Sse] struct returns mis-merged when
///   cloned (tests/regression/sse_struct_two_reg_return.c failed its first
///   value check in the paired corpus A/B, 2026-10-18).
/// * At most one natural loop: the standalone body is the shape every
///   downstream optimizer tuned; a multi-loop kernel (matmul: three nested
///   loops, 34 IR instructions) cloned into a big caller lost the
///   vectorized form and ran 2.6x slower in the paired corpus A/B
///   (2026-10-18). Multi-loop extern kernels keep their call (and their
///   optimized out-of-line body).
fn extern_inline_admission(func: &IrFunction, ctx: &CalleeContext) -> bool {
    if ctx.semantic_interposition || func.is_static {
        return false;
    }
    // Leaf helpers only: a callee that itself calls out (libm `fma@PLT`, the
    // guarded `__lccc_cpu_init` probe initialiser, Fortify wrappers) must
    // keep its call sites unique — GCC declines these too, and duplicating
    // them changed pinned contracts in the paired A/B (the fma ISA gate's
    // byte-for-byte 8-call libm count became 10; the cpu_supports runtime
    // gate's 2 initialiser calls became 4). Opaque inline asm is refused for
    // the same reason.
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. } => return false,
                _ => {}
            }
        }
    }
    if func.uses_sret
        || func.ret_is_f128_sse
        || !func.ret_eightbyte_classes.is_empty()
        || !matches!(
            func.return_type,
            IrType::I8
                | IrType::I16
                | IrType::I32
                | IrType::I64
                | IrType::I128
                | IrType::U8
                | IrType::U16
                | IrType::U32
                | IrType::U64
                | IrType::U128
                | IrType::F32
                | IrType::F64
                | IrType::Ptr
        )
    {
        return false;
    }
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let raw = loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    loop_analysis::merge_loops_by_header(raw).len() <= 1
}

fn callee_data_for(
    func: &IrFunction,
    module: &IrModule,
    ctx: &CalleeContext,
) -> Option<CalleeData> {
    let call_counts = &ctx.call_counts;
    let value_referenced = &ctx.value_referenced;
    let asm_templates = &ctx.asm_templates;
    let call_site_counts = &ctx.call_site_counts;
    let debug_callee = ctx.debug_callee;
    {
        if func.is_declaration {
            return None;
        }
        // __attribute__((noinline)) takes precedence: never inline these functions
        if func.is_noinline {
            return None;
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
            return None;
        }

        // GCC at -O2 inlines small external-linkage DEFINITIONS into callers
        // of the same translation unit while keeping the out-of-line copy:
        // the body is this TU's own code and — outside -fPIC — cannot be
        // interposed out from under a direct call (glibc's `ffs`, zlib's
        // helpers, every non-static wrapper). Admission reuses the static
        // size windows below; the single-call-site death exemption stays
        // static-only (an external body always survives for other TUs).
        //
        // ONE-LOOP CAP for external candidates: the standalone body is the
        // shape every downstream optimizer tuned; a multi-loop kernel
        // (matmul: three nested loops, 34 IR instructions) cloned into a
        // big caller lost the vectorized form and ran 2.6x slower in the
        // paired corpus A/B (2026-10-18). External admission therefore
        // covers the single-loop wrapper/small-helper class only; multi-
        // loop extern kernels keep their call (and their optimized
        // out-of-line body).
        let extern_admissible = extern_inline_admission(func, ctx);
        let is_static_like = func.is_static || extern_admissible;

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
                    .any(|succ| label_to_order.get(succ).is_some_and(|&j| j <= i))
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
        let is_small_static = is_static_like
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
        let medium_block_limit = if is_static_like && !func.is_inline {
            MAX_MEDIUM_STATIC_INLINE_BLOCKS
        } else if has_loops {
            MAX_INLINE_BLOCKS
        } else {
            MAX_INLINE_BLOCKS_NO_LOOPS
        };
        // Constant-trip tier (see MAX_CONST_TRIP_COUNT): a plain static loop
        // helper whose loop exit is bounded by a parameter may be cloned at a
        // site that passes a small constant for that parameter. The mask is
        // computed only for candidates inside the size window, so the common
        // path pays no analysis cost.
        // `always_inline` and gnu_inline bodies keep their own admission path:
        // they must be inlined at every site, so they never become
        // constant-trip-only.
        let loop_bound_params = if is_static_like
            && !func.is_inline
            && !func.is_always_inline
            && !func.is_gnu_inline_def
            && has_loops
            && inst_count_for_static <= MAX_CONST_TRIP_INLINE_INSTRUCTIONS
            && func.blocks.len() <= MAX_CONST_TRIP_INLINE_BLOCKS
        {
            loop_bound_param_mask(func)
        } else {
            0
        };
        let const_trip_candidate = loop_bound_params != 0;
        let has_outgoing_calls = func.blocks.iter().any(|b| {
            b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::Call { .. } | Instruction::CallIndirect { .. }
                )
            })
        });
        let const_branch_params = if func.is_static
            && func.is_inline
            && !func.is_always_inline
            && !func.is_gnu_inline_def
            && !has_loops
            && !has_outgoing_calls
            && inst_count_for_static <= MAX_CONST_BRANCH_INLINE_INSTRUCTIONS
            && func.blocks.len() <= MAX_CONST_BRANCH_INLINE_BLOCKS
        {
            const_branch_param_mask(func)
        } else {
            0
        };
        let const_branch_candidate = const_branch_params != 0;
        // Medium static callees up to MAX_MEDIUM_STATIC_INLINE_INSTRUCTIONS
        // instructions are admitted, including small loop CFGs. This exposes
        // aggregate-return helpers such as make_group to scalar cleanup. The
        // per-caller budget and caller-size cap remain the code-growth bounds.
        let is_medium_static = is_static_like
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
            && !const_trip_candidate
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
            return None;
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
                    return None;
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
        let effective_block_limit = if is_static_like && !func.is_inline {
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
        let fits_general_normal = fits_normal_inline_limits(
            inst_count,
            func.blocks.len(),
            is_static_like,
            func.is_inline,
            func_has_vector_intrinsics(func),
            effective_block_limit,
        ) || (is_static_like
            && !func.is_inline
            && has_loops
            && fits_static_loop_inline_limits(inst_count, func.blocks.len(), direct_call_count))
            // GNU89 `extern inline __attribute__((gnu_inline))` bodies exist
            // ONLY for inlining (no out-of-line copy is emitted in this TU);
            // a rejected site leaves a call to an external symbol that may
            // not exist (glibc rtld __bsearch). Dedicated caps: big enough
            // for glibc's header bodies, still bounded.
            || (func.is_gnu_inline_def && inst_count <= 128 && func.blocks.len() <= 16);
        // A body admitted only by the constant-trip tier is not a general
        // normal-limit callee: selection restricts it to qualifying sites.
        // Its general admission must be ignored here. The medium-static path
        // in `fits_normal_inline_limits` would otherwise admit it at every
        // site, which the early skip gate never intended for loop helpers.
        let const_trip_only = const_trip_candidate
            && !is_medium_static
            && !is_small_static
            && !fits_single_call_site_static;
        let const_branch_only =
            const_branch_candidate && !fits_general_normal && !fits_single_call_site_static;
        let fits_normal = const_trip_only || const_branch_only || fits_general_normal;
        let fits_relaxed = inst_count <= MAX_ALWAYS_INLINE_INSTRUCTIONS
            && func.blocks.len() <= MAX_ALWAYS_INLINE_BLOCKS;
        // Single-call-site statics were already size-checked above against
        // their own limits; treat them as fitting normal limits so they are
        // inlinable into ordinary callers (not just section-attributed ones).
        let exceeds_normal = !is_always_inline && !fits_normal && !fits_single_call_site_static;
        if is_always_inline {
            if inst_count > ALWAYS_INLINE_GUARD_INSTRUCTIONS
                || func.blocks.len() > ALWAYS_INLINE_GUARD_BLOCKS
            {
                return None;
            }
        } else {
            // For static inline: admit if within normal limits OR within relaxed limits
            // (the latter only used for section-attributed callers).
            if !fits_normal && !fits_single_call_site_static && !fits_relaxed {
                return None;
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
            return None;
        }

        // Don't inline functions whose static local variables contain label
        // address references (&&label). The label references in static data are
        // stored as assembly label strings (e.g., ".L3") and are NOT remapped
        // when block IDs are remapped during inlining. This causes dangling
        // references to non-existent labels, resulting in linker errors.
        if func_has_static_locals_with_label_refs(module, &func.name) {
            return None;
        }

        // Clone the function's blocks for use during inlining
        let max_block_id = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);

        let param_struct_sizes: Vec<Option<usize>> =
            func.params.iter().map(|p| p.struct_size).collect();

        // Frame bytes duplicated at every inline site. Only allocas large
        // enough to survive mem2reg/SROA count; the rest become registers.
        let agg_frame_bytes: usize = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| match inst {
                Instruction::Alloca { size, .. } if *size >= AGG_FRAME_SLOT_BYTES => Some(*size),
                _ => None,
            })
            .sum();

        Some(CalleeData {
            blocks: func.blocks.clone(),
            target_features: func.target_features.clone(),
            x86_isa_needs: func.x86_isa_needs.clone(),
            uses_fp_or_simd: body_uses_fp_or_simd(func),
            builtin_policy: func.pass_policy.builtins,
            ssp: func.ssp,
            param_struct_sizes,
            param_alloca_values: func.param_alloca_values.clone(),
            param_home_shapes: func.params.iter().map(param_home_shape).collect(),
            return_type: func.return_type,
            num_params: func.params.len(),
            // Post-structural inlining can clone values into a function
            // whose cached next_value_id predates the clone. Preserve the
            // stronger of the cached cursor and the real IR maximum so a
            // later inline cannot collide with an existing definition.
            next_value_id: std::cmp::max(func.next_value_id, func.max_value_id().saturating_add(1)),
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
            loop_bound_params,
            const_trip_only,
            const_branch_params,
            const_branch_only,
            has_inlineable_loop_descendant: false,
            has_inlined_calls: func.has_inlined_calls,
            // Any static callee with a single call site is dead after
            // inlining (internal linkage + address never taken), so it is
            // exempt from the soft caller-size cap and the per-caller
            // budget in select_inline_site (regardless of which size
            // bucket above admitted it). This deliberately INCLUDES
            // `static inline`: inlining the only call is always a net
            // code shrink (the outlined body, prologue/epilogue and call
            // sequence all disappear). Excluding `inline` here left
            // loop-containing `static inline` helpers over the 6-block
            // normal limit permanently outlined (zstd_count: 10 blocks,
            // 63 instructions, one hot call site — GCC inlines it).
            is_single_call_site_static: func.is_static
                && has_single_call_site
                && !survives_via_reference,
            single_call_site: has_single_call_site,
            agg_frame_bytes,
            has_loops,
            is_recursive,
            has_vector_intrinsics: func_has_vector_intrinsics(func),
            // Filled from the complete map below, after every eligible
            // descendant is known.
            size_inline_cost: inst_count,
            va_arg_pack: va_arg_pack_plan.take(),
        })
    }
}

/// Scalar cleanup rounds for a refreshed snapshot (each round runs the six
/// passes below; it stops at the first round that changes nothing).
const SNAPSHOT_CLEANUP_ROUNDS: usize = 3;

/// The processed body as every later caller sees it once its own post-inline
/// scalar cleanup has run: constant folding, copy propagation, SCCP, CFG
/// simplification, algebraic simplification and DCE, on a clone (the module
/// function itself is untouched). `None` when cleanup could change something
/// the inliner must preserve verbatim: inline asm (its operand symbols are
/// resolved after inlining by tracing the original value chains, see
/// `resolve_inline_asm_symbols`), label addresses or indirect branches (block
/// identities are observable), and static locals holding label addresses.
/// Whether the scalar cleanup below must leave `func` alone: inline asm
/// operands, label addresses and indirect branches pin the exact value and
/// block identities later passes (`resolve_inline_asm_symbols`, computed
/// goto) resolve, and a static local holding a label address ties the
/// function to its block labels.
fn scalar_cleanup_blocked(func: &IrFunction, module: &IrModule) -> bool {
    let pinned_blocks_or_asm = func.blocks.iter().any(|block| {
        matches!(block.terminator, Terminator::IndirectBranch { .. })
            || block.instructions.iter().any(|inst| {
                matches!(
                    inst,
                    Instruction::InlineAsm { .. } | Instruction::LabelAddr { .. }
                )
            })
    });
    pinned_blocks_or_asm || func_has_static_locals_with_label_refs(module, &func.name)
}

/// The function-local part of the post-inline cleanup chain, shared by
/// snapshot sizing and capped-caller re-measurement. It first promotes the
/// homes inlining introduced (each inlined callee's parameter homes and
/// locals): the passes below cannot see through a home, so without it a
/// cleaned body kept every clone's store/load traffic (zstd's
/// FSE_encodeSymbol: 47 raw, 83 "cleaned", 59 with the promotion). The
/// function's OWN parameter homes -- the first `params.len()` entry
/// allocas, the positional contract the inliner's argument binding and the
/// backend's `find_param_alloca` rely on -- stay, so sizes remain in the
/// home form of the raw snapshots they are compared with. Promoting them
/// too (`promote_params = true`) measured +13..+21 % zstd Ir, with calls to
/// ERR_isError, MEM_readLEST and BIT_readBitsFast left behind in
/// zstd_decompress_block.c.
fn scalar_cleanup(body: &mut IrFunction) {
    crate::ir::mem2reg::promote::promote_function(body, false);
    for _ in 0..SNAPSHOT_CLEANUP_ROUNDS {
        let changed = super::constant_fold::fold_function(body)
            + super::copy_prop::propagate_copies(body)
            + super::sccp::run_function(body)
            + super::cfg_simplify::run_function(body)
            + super::simplify::simplify_function(body)
            + super::dce::eliminate_dead_code(body);
        if changed == 0 {
            break;
        }
    }
}

fn cleaned_snapshot_body(func: &IrFunction, module: &IrModule) -> Option<IrFunction> {
    if scalar_cleanup_blocked(func, module) {
        return None;
    }
    let mut body = func.clone();
    scalar_cleanup(&mut body);
    Some(body)
}

fn refresh_callee_snapshot(
    module: &IrModule,
    func_idx: usize,
    ctx: &CalleeContext,
    callee_map: &mut FxHashMap<String, CalleeData>,
    cleanup: bool,
) {
    let func = &module.functions[func_idx];
    let instructions =
        |blocks: &[BasicBlock]| -> usize { blocks.iter().map(|b| b.instructions.len()).sum() };
    let Some(previous) = callee_map
        .get(&func.name)
        .map(|old| instructions(&old.blocks))
    else {
        return;
    };
    // A `__builtin_va_arg_pack()` wrapper means something only per call site:
    // its pack sentinels are spliced with each site's extra arguments. Its
    // processed body inlined the pack's consumer with the unresolved sentinel
    // counted as ONE argument (`__builtin_va_arg_pack_len()` folded to a
    // constant), so it is not the function's meaning and must never replace
    // the raw snapshot. Cleanup made this reachable by shrinking such a body
    // below its raw size (tests/regression/va_arg_pack_len_inline.c,
    // nested_count: 421 expected, wrong count returned).
    if callee_map
        .get(&func.name)
        .is_some_and(|old| old.va_arg_pack.is_some())
    {
        return;
    }
    // Processing leaves the body unsimplified: the inlined clones keep their
    // parameter homes, copies, folded-away branches and dead values until the
    // later scalar passes, so BIT_flushBits measured 126 instructions for 43
    // raw ones. Size, judge and clone the body after that cleanup instead
    // (the identical eligibility rules then see what callers really pay).
    let cleaned = if cleanup {
        cleaned_snapshot_body(func, module)
    } else {
        None
    };
    let Some(data) = callee_data_for(cleaned.as_ref().unwrap_or(func), module, ctx) else {
        return;
    };
    // A grown multi-site body would be cloned, grown, at every site, and
    // could push hot static-inline helpers out of their size class. Refresh
    // only when cloning cannot duplicate work (a single-call-site static,
    // held by a function that is never cloned itself, dies at that one site)
    // or when the (cleaned) processed body is no larger than the raw
    // snapshot: every clone is then no larger than the raw clone it replaces,
    // which would still have to expand the calls processing removed.
    // A single entry call site still duplicates when the function holding it
    // is itself an inline candidate (the site is cloned with its holder).
    let moved_once = data.is_single_call_site_static
        && !module.asm_labels.contains_key(&func.name)
        && ctx
            .sole_caller
            .get(&func.name)
            .is_some_and(|holder| !callee_map.contains_key(holder));
    if !moved_once && instructions(&data.blocks) > previous {
        return;
    }
    let alias = module
        .asm_labels
        .get(&func.name)
        .filter(|a| a.as_str() != func.name.as_str() && callee_map.contains_key(a.as_str()))
        .cloned();
    callee_map.insert(func.name.clone(), data);
    let mut visiting = FxHashSet::default();
    let cost = estimate_size_inline_cost(&func.name, callee_map, &mut visiting, 0);
    let mut visiting = FxHashSet::default();
    let has_loop = inlineable_loop_descendant(&func.name, callee_map, &mut visiting, 0);
    // The entry was inserted above and nothing in between removes entries.
    let Some(entry) = callee_map.get_mut(&func.name) else {
        return;
    };
    entry.size_inline_cost = cost;
    entry.has_inlineable_loop_descendant = has_loop;
    if let Some(alias) = alias {
        let aliased = entry.clone();
        callee_map.insert(alias, aliased);
    }
}

fn build_callee_map(module: &IrModule) -> FxHashMap<String, CalleeData> {
    build_callee_map_with(module, &callee_context(module))
}

fn build_callee_map_with(module: &IrModule, ctx: &CalleeContext) -> FxHashMap<String, CalleeData> {
    let mut map = FxHashMap::default();
    for func in &module.functions {
        let Some(data) = callee_data_for(func, module, ctx) else {
            continue;
        };
        map.insert(func.name.clone(), data);
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

/// Defer a small ordinary wrapper which hides more mandatory expansion than
/// the normal caller budget. The callee map is a pre-expansion snapshot: cloning
/// this raw wrapper AFTER the caller's phase-0 drain introduces a large forced
/// template behind the economics budgets. Those budgets may leave its runtime
/// branches/indirect calls intact. Keeping the wrapper lets its own phase-0
/// expansion and constant propagation specialize the template first.
///
/// This is a scheduling deferral, not a permanent single-call-site veto. A later
/// invocation sees the expanded loop body and uses the existing normal policy.
/// Never override explicit inline contracts, section requirements or PGO, and
/// do not change the separately measured size-optimization policy.
fn template_wrapper_should_wait_for_specialization(
    data: &CalleeData,
    raw_inst_count: usize,
    caller_has_section: bool,
    pgo_force: bool,
    size_optimized: bool,
) -> bool {
    !size_optimized
        && data.is_plain_static
        && !data.has_loops
        && data.has_inlineable_loop_descendant
        && raw_inst_count <= MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS
        && data.blocks.len() <= MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS
        && data.size_inline_cost > MAX_INLINE_BUDGET_PER_CALLER
        && !caller_has_section
        && !pgo_force
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

/// The ISA contract of inlining: a body may move only into a caller whose
/// feature set covers what the body needs. Every inline path selects sites
/// through [`find_inline_call_sites`], so this one check covers ordinary,
/// always_inline and PGO-forced inlining. Refusing is always correct: the
/// callee stays an ordinary call.
///
/// A callee with its own `target(...)` needs its whole feature set (GCC's
/// `can_inline_p`, each set read in GCC's semantics by
/// [`crate::backend::x86::isa::function_feature_bits`]). zstd's runtime
/// dispatch shows why (`ZSTD_encodeSequences` → `static
/// BMI2_TARGET_ATTRIBUTE ZSTD_encodeSequences_bmi2`, single call site): the
/// BMI2 clone inlined into its plain dispatcher would be emitted under the
/// dispatcher's baseline ISA, silently discarding the clone, and a plain
/// caller absorbing a `target("avx2")` body would execute AVX2 before the
/// CPUID check.
///
/// A callee without `target(...)` needs only what its IR depends on
/// ([`crate::ir::module::X86IsaNeeds`]): the features the lowering relied
/// on, the requirements an `always_inline` header body deferred, and the
/// FP/SIMD registers if it computes with such values. The rest of its IR is
/// lowered under the caller's own features, so `_mm_popcnt_u32` inlined
/// into a `target("arch=x86-64")` dispatch baseline becomes the portable
/// sequence there, `_mm512_add_epi8` inlines into a `target("avx512bw")`
/// caller of a `-march=x86-64` unit, and a helper that relied on AVX2
/// stays a call. A plain `always_inline` caller takes every plain body and
/// its needs: it is checked in turn where it is inlined.
fn target_isa_allows_inline(caller: &IrFunction, callee: &CalleeData) -> bool {
    match crate::common::types::target_elf_machine() {
        // 3 = EM_386 (i686), 62 = EM_X86_64.
        machine @ (3 | 62) => x86_features_allow_inline(
            &caller.target_features,
            caller.is_always_inline,
            IsaCallee {
                features: &callee.target_features,
                needs: callee.x86_isa_needs.relied | callee.x86_isa_needs.deferred,
                uses_fp_or_simd: callee.uses_fp_or_simd,
            },
            crate::common::x86_features::effective().raw(),
            crate::common::x86_features::explicit(),
            machine == 62,
        ),
        _ => callee
            .target_features
            .iter()
            .all(|f| caller.target_features.contains(f)),
    }
}

/// The callee side of [`x86_features_allow_inline`].
#[derive(Clone, Copy)]
struct IsaCallee<'a> {
    /// Its normalized `target(...)` options.
    features: &'a [String],
    /// Its relied and deferred x86 needs (`X86Feature` bits).
    needs: u64,
    uses_fp_or_simd: bool,
}

/// [`target_isa_allows_inline`] on x86 in a translation unit with features
/// `tu`, `tu_explicit` of them explicit on the command line.
fn x86_features_allow_inline(
    caller: &[String],
    caller_always_inline: bool,
    callee: IsaCallee<'_>,
    tu: u64,
    tu_explicit: u64,
    x86_64: bool,
) -> bool {
    use crate::backend::x86::isa::{function_feature_bits, function_x87_denial};
    use crate::common::x86_features::X86Feature as F;
    let features = |options: &[String]| function_feature_bits(tu, tu_explicit, options, x86_64);
    if !callee.features.is_empty() {
        return caller == callee.features || features(callee.features) & !features(caller) == 0;
    }
    if caller_always_inline && caller.is_empty() {
        return true;
    }
    let mut needed = callee.needs;
    if callee.uses_fp_or_simd {
        if function_x87_denial(caller).is_some() {
            return false;
        }
        needed |= tu & (1 << F::Sse as u8 | 1 << F::Sse2 as u8);
    }
    needed & !features(caller) == 0
}

/// Whether `func` computes with floating-point or SIMD values, so that
/// wherever it is inlined the FP/vector register file must be available.
fn body_uses_fp_or_simd(func: &IrFunction) -> bool {
    use crate::common::types::EightbyteClass;
    let call_uses_fp = |info: &crate::ir::instruction::CallInfo| {
        info.return_type.is_float()
            || info.arg_types.iter().any(IrType::is_float)
            || info
                .struct_arg_classes
                .iter()
                .flatten()
                .any(|class| matches!(class, EightbyteClass::Sse | EightbyteClass::SseUp))
    };
    func.return_type.is_float()
        || func.params.iter().any(|param| param.ty.is_float())
        || func
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|inst| {
                inst.result_type().is_some_and(|ty| ty.is_float())
                    || match inst {
                        Instruction::Store { ty, .. } => ty.is_float(),
                        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                            call_uses_fp(info)
                        }
                        Instruction::Intrinsic { op, .. } => op.is_vector_op(),
                        _ => false,
                    }
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
                        // ISA contract: never clone a body that may use
                        // extensions its caller was not compiled for.
                        if !target_isa_allows_inline(func, callee_data) {
                            continue;
                        }
                        if func.ssp.no_stack_protector != callee_data.ssp.no_stack_protector {
                            continue;
                        }
                        // Skip callees that exceed normal limits unless the caller
                        // has a section (cross-section calls are dangerous) or
                        // the callee is dead after inlining (single call site,
                        // address never taken): then inlining is a net shrink
                        // and size limits do not apply (GCC
                        // -finline-functions-called-once parity).
                        if callee_data.exceeds_normal_limits
                            && !caller_has_section
                            && !callee_data.is_single_call_site_static
                            // Hot-leaf and SROA-unlock candidates are
                            // offered here; whether such a site is
                            // admitted is decided in `select_inline_site`
                            // (`hot_inline_leaf` / `sroa_unlocking_leaf`).
                            && !is_hot_leaf_candidate(callee_data)
                            && !is_sroa_unlockable_candidate(callee_data)
                            && !is_hot_loop_helper_candidate(callee_data)
                        {
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

/// Widest local whose `memcpy` copy may be elided.
///
/// Eight bytes is the table-entry shape the transform exists for
/// (`const FSE_symbolCompressionTransform symbolTT = table[symbol]`); sixteen
/// covers a two-field struct. Wider copies are consumed by more loads than the
/// copy itself costs, so eliding them would be a pessimisation.
const ELIDE_MEMCPY_MAX_BYTES: usize = 16;

/// `(candidates: local -> size, src_of: local -> copy source, disqualified,
/// copy_at: local -> entry-block index of its copy)`.
type ElideMemcpySets = (
    FxHashMap<u32, usize>,
    FxHashMap<u32, Value>,
    FxHashSet<u32>,
    FxHashMap<u32, usize>,
);

/// Phase 1 of [`elide_readonly_local_memcpys`]: the small non-volatile
/// entry-block locals, each with exactly one full-width entry-block copy
/// (`src_of`) at a known position (`copy_at`), in a call/asm-free body.
/// `None` when nothing can be elided.
fn elide_memcpy_candidates(blocks: &[BasicBlock]) -> Option<ElideMemcpySets> {
    let Some(entry) = blocks.first() else {
        return None;
    };
    // ── candidates: non-volatile entry-block locals the copy could replace ──
    let mut candidates: FxHashMap<u32, usize> = FxHashMap::default();
    for inst in &entry.instructions {
        if let Instruction::Alloca {
            dest,
            size,
            volatile: false,
            semantic_volatile: false,
            ..
        } = inst
            && *size > 0
            && *size <= ELIDE_MEMCPY_MAX_BYTES
        {
            candidates.insert(dest.0, *size);
        }
    }
    if candidates.is_empty() {
        return None;
    }
    // A call can read or write the local through a pointer this analysis
    // cannot see (`lccc` marks nothing `restrict` here), and inline asm can do
    // anything; both end the transform for the whole body.
    if blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::InlineAsm { .. }
            )
        })
    }) {
        return None;
    }
    // ── exactly one full-width copy per candidate, from the entry block ──
    let mut src_of: FxHashMap<u32, Value> = FxHashMap::default();
    let mut disqualified: FxHashSet<u32> = FxHashSet::default();
    for inst in &entry.instructions {
        if let Instruction::Memcpy { dest, src, size } = inst
            && let Some(&alloca_sz) = candidates.get(&dest.0)
        {
            if *size == alloca_sz
                && !src_of.contains_key(&dest.0)
                && !disqualified.contains(&dest.0)
            {
                src_of.insert(dest.0, *src);
            } else {
                disqualified.insert(dest.0);
            }
        }
    }
    if src_of.is_empty() {
        return None;
    }
    // Entry-block position of each candidate's copy: the only copy into the
    // local the rewrite may delete, and the point its snapshot is taken.
    let mut copy_at: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, inst) in entry.instructions.iter().enumerate() {
        if let Instruction::Memcpy { dest, src, .. } = inst
            && src_of.get(&dest.0) == Some(src)
        {
            copy_at.insert(dest.0, idx);
        }
    }
    Some((candidates, src_of, disqualified, copy_at))
}

/// Phase 2 of [`elide_readonly_local_memcpys`]: the alias closure with
/// constant offsets. A value reached from two roots disqualifies both.
fn elide_memcpy_alias_closure(
    blocks: &[BasicBlock],
    src_of: &FxHashMap<u32, Value>,
    disqualified: &mut FxHashSet<u32>,
) -> FxHashMap<u32, (u32, Option<i64>)> {
    // ── alias closure with constant offsets, to a fixpoint ──
    //
    // `alias[v] = (root, Some(off))` when `v` is the candidate `root` plus the
    // constant byte offset `off` (through `GetElementPtr`s with constant
    // offsets and `Copy`s), `(root, None)` when an offset on the way is not a
    // constant. The walk is order-independent because block order is not
    // dominance order after inlining. A value seen with two different
    // derivations only ever loses information (`Some -> None`), so the loop
    // terminates; one reached from two roots disqualifies both.
    let mut alias: FxHashMap<u32, (u32, Option<i64>)> =
        src_of.keys().map(|&k| (k, (k, Some(0)))).collect();
    loop {
        let mut grew = false;
        for block in blocks.iter() {
            for inst in &block.instructions {
                let derived = match inst {
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => alias.get(&base.0).map(|&(root, off)| {
                        let next = match (off, offset) {
                            (Some(o), Operand::Const(c)) => {
                                c.to_i64().and_then(|k| o.checked_add(k))
                            }
                            _ => None,
                        };
                        (dest.0, (root, next))
                    }),
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(v),
                    } => alias.get(&v.0).map(|&a| (dest.0, a)),
                    _ => None,
                };
                let Some((v, a)) = derived else {
                    continue;
                };
                match alias.get(&v).copied() {
                    None => {
                        alias.insert(v, a);
                        grew = true;
                    }
                    Some(old) if old == a => {}
                    Some((old_root, old_off)) => {
                        if old_root != a.0 {
                            disqualified.insert(old_root);
                            disqualified.insert(a.0);
                        }
                        if old_off.is_some() {
                            alias.insert(v, (old_root, None));
                            grew = true;
                        }
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    alias
}

/// Elide `Instruction::Memcpy { dest, src, size }` into a small (`<= 16` byte)
/// read-only local `Alloca` inside a call-free inlined body: the local's
/// readers read `src` directly, at the point of the copy.
///
/// Hot inline leaves such as zstd's `FSE_encodeSymbol` copy an 8-byte table
/// entry into a local struct and read two 32-bit fields from it. Leaving the
/// `Memcpy` in place forces both a stack round-trip and a `call_spanning`
/// barrier in liveness/regalloc across every inlined site.
///
/// # Soundness
///
/// A reader of the local sees the bytes the copy took from `src` *when the
/// copy ran*. Re-rooting a reader at `src` where it stands would read the
/// bytes `src` holds *later*, which differs as soon as anything writes the
/// source in between (a field store, a store through an aliasing pointer, a
/// loop-carried update, the caller's object behind a by-value parameter).
/// So every reader (`GetElementPtr` and `Copy` of the local, and the loads
/// through them) is MOVED to just after the copy and reads `src` there:
/// the same bytes at the same instant, whatever happens afterwards. This is
/// scalar replacement of the copy, as GCC's SRA does it.
///
/// That move is valid when:
///
/// * the copy is in the entry block, which dominates the whole body, and
///   every operand of a moved instruction exists there: `GetElementPtr`
///   offsets are constants, and no load of the local precedes the copy;
/// * each load reads a constant, in-bounds sub-range of the copied bytes with
///   no segment override: a moved load may run on a path where the original
///   did not, which cannot fault only on bytes the copy itself dereferenced;
/// * the local is touched by nothing but its readers and that one copy. A
///   write through an alias, a volatile read, any other use of the address
///   (stored, returned, passed, `Select`ed, ...) or a second copy into it
///   (matched by position, even from the same source value) disqualifies it;
/// * the body has no call, indirect call or inline asm.
///
/// Copy chains (`L2 <- L1 <- p`, the inliner's double buffer for by-value
/// parameters) resolve to the ultimate source and are snapshotted at the
/// chain's first copy; a copy that survives while reading an elided local
/// keeps that local instead. The alias closure is computed to a fixpoint
/// before anything is judged, because `BasicBlock` order is not dominance
/// order after inlining.
fn elide_readonly_local_memcpys(blocks: &mut [BasicBlock]) {
    let Some((candidates, src_of, mut disqualified, copy_at)) = elide_memcpy_candidates(blocks)
    else {
        return;
    };
    let alias = elide_memcpy_alias_closure(blocks, &src_of, &mut disqualified);
    let root_of = |v: u32| alias.get(&v).map(|&(root, _)| root);
    // ── escape scan: the local must be touched by nothing but its readers ──
    for (bi, block) in blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                // Definitions and alias producers. An alias producer is moved
                // to the copy, so every operand it has must exist there: a
                // `GetElementPtr` of an alias needs a constant offset.
                Instruction::Alloca { .. }
                | Instruction::Copy { .. }
                | Instruction::Fence { .. } => {}
                Instruction::GetElementPtr { base, offset, .. } => {
                    if let Some(root) = root_of(base.0)
                        && !matches!(offset, Operand::Const(_))
                    {
                        disqualified.insert(root);
                    }
                    if let Operand::Value(v) = offset
                        && let Some(root) = root_of(v.0)
                    {
                        disqualified.insert(root);
                    }
                }
                // A non-volatile read of the local: moved to the copy and
                // re-rooted at the source. It must read a constant, in-bounds
                // sub-range of the copied bytes (the hoisted load may now run
                // on a path the original did not take, which is only safe on
                // bytes the copy itself dereferenced), with no segment
                // override, and it must not precede the copy.
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override,
                    volatile: false,
                    ..
                } => {
                    if let Some(&(root, off)) = alias.get(&ptr.0) {
                        let size = candidates.get(&root).copied().unwrap_or(0) as i64;
                        let in_bounds = off.is_some_and(|o| {
                            o >= 0
                                && ty.access_extent().is_some_and(|(_, hi)| {
                                    o.checked_add(hi).is_some_and(|end| end <= size)
                                })
                        });
                        let after_copy = bi != 0 || copy_at.get(&root).is_some_and(|&c| ii > c);
                        if !in_bounds || *seg_override != AddressSpace::Default || !after_copy {
                            disqualified.insert(root);
                        }
                    }
                }
                // The candidate's own copy, by position: a second copy into
                // the local (even from the same source value) refills it with
                // newer bytes and is an escape like any other write.
                Instruction::Memcpy { dest, src, .. }
                    if bi == 0 && copy_at.get(&dest.0) == Some(&ii) =>
                {
                    // Its source may be another candidate (a copy chain) or an
                    // alias of one at an offset; only the former is modelled.
                    if let Some(root) = root_of(src.0)
                        && root != src.0
                    {
                        disqualified.insert(dest.0);
                        disqualified.insert(root);
                    }
                }
                // Everything else: an alias in any operand is an escape. This
                // includes writes through an alias, whose destination vanishes.
                _ => inst.for_each_used_value(|v| {
                    if let Some(root) = root_of(v) {
                        disqualified.insert(root);
                    }
                }),
            }
        }
        block.terminator.for_each_used_value(|v| {
            if let Some(root) = root_of(v) {
                disqualified.insert(root);
            }
        });
    }
    // ── settle the elided set: chains, ordering and surviving copies ──
    //
    // A candidate's source can itself be a candidate: `memcpy(a, b)` where `b`
    // has its own copy `memcpy(b, p)`. The inliner builds exactly those
    // double-buffer chains for by-value struct parameters (`L2 home <- L1 home
    // <- param pointer`). Every chain resolves to its ultimate source and is
    // snapshotted at its FIRST copy (the one reading that source): `a` holds
    // the bytes `b` took from `p` then, whatever `p` holds later. A later link
    // must follow the earlier one in the entry block. A cycle has no ultimate
    // source and is left alone. A copy that survives (its destination is
    // kept) but reads an elided local would have to read the source late, so
    // that local is kept too; dropping a local can break other chains, hence
    // the fixpoint.
    let mut elide: FxHashSet<u32> = src_of
        .keys()
        .copied()
        .filter(|k| !disqualified.contains(k))
        .collect();
    let mut plan: FxHashMap<u32, (Value, usize)> = FxHashMap::default();
    loop {
        plan.clear();
        for &cand in &elide {
            let mut cur = cand;
            let mut seen: FxHashSet<u32> = FxHashSet::default();
            let mut ok = true;
            // Walk to the ultimate source; positions must decrease.
            let (src, at) = loop {
                let src = src_of[&cur];
                let at = copy_at[&cur];
                if !elide.contains(&src.0) {
                    break (src, at);
                }
                if !seen.insert(cur) || src.0 == cand || copy_at[&src.0] >= at {
                    ok = false;
                    break (src, at);
                }
                cur = src.0;
            };
            if ok {
                plan.insert(cand, (src, at));
            }
        }
        let mut next: FxHashSet<u32> = plan.keys().copied().collect();
        for (d, s) in &src_of {
            if !next.contains(d)
                && let Some(root) = root_of(s.0)
            {
                next.remove(&root);
            }
        }
        if next == elide {
            break;
        }
        elide = next;
    }
    plan.retain(|k, _| elide.contains(k));
    if plan.is_empty() {
        return;
    }
    // ── rewrite: move every reader to its snapshot point, re-rooted ──
    //
    // Readers are `GetElementPtr`, `Copy` and non-volatile `Load` of an alias.
    // Each is removed from its block, its root operand (the local itself) is
    // replaced by the ultimate source, and the group is re-inserted where the
    // chain's first copy was, in dependency order. Every reader then reads the
    // source at the instant the copy did, so no later write to the source -- a
    // store, an aliasing pointer, a loop-carried update -- can reach it. The
    // entry block dominates the whole body, and every operand of a moved
    // instruction (the source, a constant, an earlier moved value) is defined
    // at that point, so each moved value still dominates all of its uses.
    let elided_root = |v: u32| root_of(v).filter(|r| plan.contains_key(r));
    // Each moved reader keeps its own debug location when its block has a
    // parallel span vector (the line that names the field is the useful one
    // to step to); otherwise it takes the copy's.
    let mut moved: FxHashMap<usize, Vec<(u32, Instruction, Option<crate::common::source::Span>)>> =
        FxHashMap::default();
    // The entry block is rebuilt last: groups pulled from later blocks are
    // inserted into it. `None` marks a removed slot.
    let mut entry_slots: Vec<Option<Instruction>> = Vec::new();
    for (bi, block) in blocks.iter_mut().enumerate() {
        let parallel =
            !block.source_spans.is_empty() && block.source_spans.len() == block.instructions.len();
        let old_spans = std::mem::take(&mut block.source_spans);
        let mut kept: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
        let mut spans = Vec::with_capacity(old_spans.len());
        for (idx, mut inst) in block.instructions.drain(..).enumerate() {
            // Decide first, move afterwards: (drop, Some((root, used)) if moved).
            let (drop, mv) = match &mut inst {
                Instruction::Alloca { dest, .. } | Instruction::Memcpy { dest, .. }
                    if plan.contains_key(&dest.0) =>
                {
                    (true, None)
                }
                Instruction::GetElementPtr { base: p, .. }
                | Instruction::Load { ptr: p, .. }
                | Instruction::Copy {
                    src: Operand::Value(p),
                    ..
                } => match elided_root(p.0) {
                    Some(root) => {
                        let used = p.0;
                        if let Some(&(src, _)) = plan.get(&used) {
                            *p = src;
                        }
                        (true, Some((root, used)))
                    }
                    None => (false, None),
                },
                _ => (false, None),
            };
            if let Some((root, used)) = mv {
                let span = parallel.then(|| old_spans[idx]);
                moved
                    .entry(plan[&root].1)
                    .or_default()
                    .push((used, inst, span));
                if bi == 0 {
                    entry_slots.push(None);
                }
                continue;
            }
            if bi == 0 {
                entry_slots.push((!drop).then_some(inst));
                continue;
            }
            if drop {
                continue;
            }
            kept.push(inst);
            if parallel {
                spans.push(old_spans[idx]);
            }
        }
        if bi == 0 {
            block.source_spans = old_spans;
            continue;
        }
        block.instructions = kept;
        // Never drop a span vector: a vector that does not parallel the
        // instruction list is better left as-is than discarded (imprecise
        // debug locations beat none).
        block.source_spans = if parallel { spans } else { old_spans };
    }
    let entry = &mut blocks[0];
    let parallel = !entry.source_spans.is_empty() && entry.source_spans.len() == entry_slots.len();
    let old_spans = std::mem::take(&mut entry.source_spans);
    let mut insts: Vec<Instruction> = Vec::with_capacity(entry_slots.len());
    let mut spans = Vec::with_capacity(old_spans.len());
    for (idx, slot) in entry_slots.into_iter().enumerate() {
        if let Some(inst) = slot {
            insts.push(inst);
            if parallel {
                spans.push(old_spans[idx]);
            }
        }
        if let Some(group) = moved.remove(&idx) {
            for (inst, span) in order_moved_readers(group, &plan) {
                insts.push(inst);
                if parallel {
                    spans.push(span.unwrap_or(old_spans[idx]));
                }
            }
        }
    }
    debug_assert!(
        moved.is_empty(),
        "every snapshot point is an entry-block copy"
    );
    entry.instructions = insts;
    entry.source_spans = if parallel { spans } else { old_spans };
    // Machine-checked post-condition: nothing may still reference a local or
    // copy that no longer exists. Debug builds pay one linear scan; the
    // invariant is exactly what a future edit to this function could break.
    debug_assert!(
        blocks.iter().all(|b| {
            let insts_clean = b.instructions.iter().all(|i| {
                let mut clean = true;
                i.for_each_used_value(|v| clean &= !plan.contains_key(&v));
                clean
            });
            let mut term_clean = true;
            b.terminator
                .for_each_used_value(|v| term_clean &= !plan.contains_key(&v));
            insts_clean && term_clean
        }),
        "elided memcpy local still referenced after rewrite"
    );
}

/// Order one snapshot group of moved readers so every value is defined before
/// it is used. `used` is the alias operand each reader consumed: a root (the
/// elided local, now replaced by its source) or another moved reader's value.
/// SSA makes the alias graph a forest rooted at the elided locals, so every
/// round emits at least one reader.
fn order_moved_readers<S>(
    group: Vec<(u32, Instruction, S)>,
    roots: &FxHashMap<u32, (Value, usize)>,
) -> Vec<(Instruction, S)> {
    let mut defined: FxHashSet<u32> = FxHashSet::default();
    let mut out = Vec::with_capacity(group.len());
    let mut pending = group;
    while !pending.is_empty() {
        let before = pending.len();
        let mut rest = Vec::new();
        for (used, inst, extra) in pending {
            if roots.contains_key(&used) || defined.contains(&used) {
                if let Some(d) = inst.dest() {
                    defined.insert(d.0);
                }
                out.push((inst, extra));
            } else {
                rest.push((used, inst, extra));
            }
        }
        assert!(rest.len() < before, "moved readers form a cycle");
        pending = rest;
    }
    out
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

/// Inline one level of a *directly self-recursive* callee into itself.
///
/// GCC expands a self call once (`--param max-inline-recursive-depth-auto`),
/// so a full tree walk in a body that makes two self calls per node pays half
/// the call/return pairs, prologues and epilogues per level and needs half the
/// frame depth. LCCC refused every recursive callee instead
/// (`select_inline_site`, both passes, plus `find_inline_call_sites`), which is
/// the correct default for a *fixed-point* inliner — a clone still contains the
/// recursive call, so repeating the expansion only inflates the caller — but it
/// leaves the single, bounded level on the table. `binary_trees`
/// (`make`/`check`/`destroy` over 2^d nodes) is the shape that measures the
/// difference: LCCC/GCC 1.14 on the CI runner, 1.29 locally.
///
/// The expansion is bounded, not fixed-point:
///
/// * within one invocation the site list is collected once, from the entry
///   body, and processed in reverse program order (`inline_call_site` inserts
///   the clone *after* the split block, so lower-index and lower-instruction
///   sites stay valid) — the clones' own calls are never revisited;
/// * across invocations the body has to fit `MAX_SELF_CLONE_INSTRUCTIONS`,
///   `MAX_SELF_CLONE_BLOCKS`, `MAX_SELF_CLONE_SITES` *and*
///   `MAX_SELF_CLONE_FINAL_INSTRUCTIONS`, and every expansion strictly grows
///   the body, so the sequence terminates;
/// * a body with `N` self call sites comes back with `N^2`, so the tree shape
///   (`N = 2`) is exactly one level (`4 > MAX_SELF_CLONE_SITES`) while a
///   single-site body may expand again while it still fits — GCC's
///   `max-inline-recursive-depth-auto` behaves the same way;
/// * only a *direct* self call qualifies, never mutual recursion, so the frame
///   growth stays inside one function;
/// * `DynAlloca` and aggregate frame bytes are refused: the clone must not
///   multiply dynamic stack use, only one static level;
/// * the body must be small, branch-light and end under
///   `MAX_SELF_CLONE_FINAL_INSTRUCTIONS`.
///
/// Static locals are *not* a hazard here (unlike ordinary inlining): both
/// copies are the same function, so they name the same object.
fn clone_self_recursion_sites(
    module: &mut IrModule,
    callee_map: &FxHashMap<String, CalleeData>,
    skip_list: &[String],
    global_max_block_id: &mut u32,
    debug_inline: bool,
) -> usize {
    let mut cloned = 0usize;
    for func_idx in 0..module.functions.len() {
        let func = &module.functions[func_idx];
        if func.is_declaration
            || func.is_always_inline
            || func.is_noinline
            || func.section.is_some()
            || skip_list.iter().any(|s| s == &func.name)
            || was_self_inlined(&func.name)
        {
            continue;
        }
        let Some(data) = callee_map.get(&func.name) else {
            continue;
        };
        let entry_size: usize = func.blocks.iter().map(|b| b.instructions.len()).sum();

        if entry_size == 0
            || entry_size > MAX_SELF_CLONE_INSTRUCTIONS
            || data.blocks.len() > MAX_SELF_CLONE_BLOCKS
            || data.agg_frame_bytes != 0
        {
            continue;
        }
        // A *dynamic* stack object would be multiplied by the clone: the frame
        // growth must stay one static level. A small `Alloca` is a promotable
        // parameter slot (mem2reg/SROA runs after this pass); a large one is
        // already refused above through `agg_frame_bytes`.
        if data.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::DynAlloca { .. }))
        }) {
            continue;
        }
        // Collect the entry body's self call sites, in program order.
        let mut sites: Vec<InlineCallSite> = Vec::new();
        let name = func.name.clone();
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                if let Instruction::Call { func: callee, info } = inst {
                    if callee == &name {
                        sites.push(InlineCallSite {
                            block_idx,
                            inst_idx,
                            callee_name: name.clone(),
                            dest: info.dest,
                            args: info.args.clone(),
                            pgo_force: false,
                            arg_meta: CallArgMeta::from_call_info(info),
                        });
                    }
                }
            }
        }
        if sites.is_empty()
            || sites.len() > MAX_SELF_CLONE_SITES
            || entry_size + sites.len() * entry_size > MAX_SELF_CLONE_FINAL_INSTRUCTIONS
        {
            continue;
        }
        // A *tail* self call belongs to `tail_call_elim`: it becomes a back
        // edge, and the loop it creates is what `closed_form_tail_sum` and
        // LICM/IVSR then work on. Cloning it first would duplicate the body
        // into a shape neither of those recognises (measured: `tce_sum`'s
        // `sum(n, acc)` stopped folding to its closed form), so leave every
        // function with a tail-position self call entirely to TCE.
        if sites.iter().any(|site| {
            let block = &func.blocks[site.block_idx];
            match (&block.terminator, site.dest) {
                (Terminator::Return(None), None) => true,
                (Terminator::Return(Some(Operand::Value(rv))), Some(dest)) => rv == &dest,
                _ => false,
            }
        }) {
            continue;
        }
        // Reverse program order: each inline inserts its clone blocks directly
        // after the split block, so every not-yet-processed site keeps its
        // (block_idx, inst_idx).
        sites.sort_by_key(|s| (s.block_idx, s.inst_idx));
        sites.reverse();
        let mut inlined_here = 0usize;
        for site in &sites {
            let ok = inline_call_site(
                &mut module.functions[func_idx],
                site,
                data,
                global_max_block_id,
            );
            if ok {
                inlined_here += 1;
            }
        }
        if inlined_here == 0 {
            continue;
        }
        cloned += inlined_here;
        // The body now contains expanded code from here on; the wrapper
        // heuristics that read this flag must see that.
        module.functions[func_idx].has_inlined_calls = true;
        if debug_inline {
            eprintln!(
                "[INLINE] Cloned one level of self-recursion in '{}' ({} site(s), {} instructions)",
                module.functions[func_idx].name, inlined_here, entry_size
            );
        }
    }
    cloned
}

/// Bytes a parameter home must own to hold a value of `ty`: the widest extent
/// an access of that type may touch, and never less than a word (the front end
/// gives scalars a word-sized slot).  `Alloca` `size` is what the stack
/// layout reserves, while the bridge stores with the home's `ty` and the
/// `ParamRef` reads with its own -- a slot narrower than the value truncates it,
/// and a wider store than the slot overwrites the slots after it.  A vector or
/// `__float128` parameter is 16 bytes, a scalar one word.
fn ir_type_slot_size(ty: IrType) -> usize {
    let may = ty.access_extent().map_or(16, |(_, may)| may as usize);
    may.max(8)
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
    // Labels are handed out above every label in the module *and* above the
    // caller's own maximum: `global_max_block_id` is a maintained counter, and
    // structural transforms elsewhere can mint labels from a function's
    // `next_label` without advancing it (measured here: the caller held
    // 4189138786 while the counter stood at 4093052108).  Taking the maximum of
    // both is what makes the dense assignment collision-free by construction.
    let caller_max_label = caller
        .blocks
        .iter()
        .map(|block| block.label.0)
        .max()
        .unwrap_or(0);
    // Capacity: the clone mints one label per callee block plus the reserved
    // extras, so that is what the base must leave room for. Checked before any
    // block is cloned; an inline that cannot fit refuses (the caller keeps the
    // call) instead of minting a label that already exists.
    let label_need = (callee.blocks.len() as u64).saturating_add(LABEL_MAP_RESERVED as u64);
    let Some(label_base) = (*global_max_block_id)
        .max(caller_max_label)
        .checked_add(1)
        .filter(|base| u64::from(*base) + label_need <= u64::from(u32::MAX))
    else {
        return false;
    };

    let debug_inline_detail = std::env::var("CCC_INLINE_DEBUG_DETAIL").is_ok();
    if debug_inline_detail {
        eprintln!(
            "[INLINE_DETAIL] Inlining '{}' into '{}': value_offset={}, label_base={}, callee.next_value_id={}, caller.next_value_id={}",
            site.callee_name,
            caller.name,
            value_offset,
            label_base,
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

    // Clone and remap the callee's blocks.
    //
    // Block labels are remapped DENSELY, through `labels`, never by adding a
    // constant offset to the callee's own labels.  Additive re-basing is
    // exponential: every clone's labels sit above the module-wide maximum, and
    // the next clone adds that maximum again, so a function that inlines a few
    // times per optimization round drives the u32 label space to its top.
    // Measured on linux-6.18.58 kernel/workqueue.c: `process_scheduled_works`
    // inlined `process_one_work` (whose own labels had been rebased below
    // 4.09e9), then the next clones in that caller reused the same base and one
    // landed back on an existing label by u32 wraparound -- two live blocks
    // with one label.  `cfg-simplify` then merged the wrong blocks, deleted the
    // definition of v277 and left five dangling uses (observed as
    // `x86 codegen: operand_to_rax: value 277 ... cannot be materialised`):
    // a miscompile, not a diagnostic.
    //
    // Dense assignment keeps label growth linear in the number of cloned blocks
    // and makes collisions structurally impossible: labels are handed out from
    // a counter above every label in the module (and above the caller's own
    // highest label), and each distinct callee label consumes exactly one slot.
    let mut labels = LabelMap::new(label_base);
    for callee_block in &callee.blocks {
        labels.assign(callee_block.label);
    }

    let mut inlined_blocks: Vec<BasicBlock> = Vec::with_capacity(callee.blocks.len());

    for callee_block in &callee.blocks {
        let mut new_block = BasicBlock {
            label: labels.get(callee_block.label),
            instructions: Vec::with_capacity(callee_block.instructions.len()),
            source_spans: callee_block.source_spans.clone(),
            terminator: remap_terminator(&callee_block.terminator, value_offset, &mut labels),
        };

        for inst in &callee_block.instructions {
            new_block
                .instructions
                .push(remap_instruction(inst, value_offset, &mut labels));
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
                block.retain_insts(|inst| {
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

        // Zero extra arguments is a valid count too. The plan contains
        // original callee IDs, but inlined_blocks have ALREADY been remapped.
        if !plan.len_values.is_empty() {
            let len_const = IrConst::I32(extra.len() as i32);
            for block in &mut inlined_blocks {
                for inst in &mut block.instructions {
                    if let Instruction::Call { func, info } = inst {
                        if func == "__lccc_va_arg_pack_len" {
                            if let Some(d) = info.dest {
                                if plan.len_values.iter().any(|v| v.0 + value_offset == d.0) {
                                    // Keep the already-remapped destination: adding the
                                    // offset again leaves every use undefined (VAPACK-LEN-1).
                                    *inst = Instruction::Copy {
                                        dest: d,
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

    // Create a merge block that the callee's return statements will branch to.
    // Its label comes from the same dense allocator, so it is above every label
    // the clone minted and above anything the caller already had — including the
    // labels of any later clone in this caller that mints from `next_label`.
    let merge_block_id = labels.assign_fresh();

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

    // Resolve homes by lowering's stable identity record, never alloca order
    // or a store to an unrelated same-typed local. Missing live homes are
    // synthesized with the ABI shape; dead parameters require no bridge.
    let Some(mut param_alloca_info) = inlined_param_homes(callee, &inlined_blocks, value_offset)
    else {
        return false;
    };
    let mut paramref_owner: FxHashMap<u32, (usize, IrType)> = FxHashMap::default();
    let mut clone_max = 0u32;
    for block in &inlined_blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                clone_max = clone_max.max(dest.0);
            }
            inst.for_each_used_value(|id| clone_max = clone_max.max(id));
            if let Instruction::ParamRef {
                dest,
                param_idx,
                ty,
            } = inst
            {
                paramref_owner.insert(dest.0, (*param_idx, *ty));
            }
        }
        block
            .terminator
            .for_each_used_value(|id| clone_max = clone_max.max(id));
    }
    let Some(next_fresh) = clone_max
        .checked_add(1)
        .zip(value_offset.checked_add(callee.next_value_id))
        .map(|(actual, cached)| actual.max(cached))
    else {
        return false;
    };
    let mut next_fresh = next_fresh;
    let mut synthesized = Vec::new();
    for (idx, home) in param_alloca_info.iter_mut().enumerate() {
        if home.is_some() {
            continue;
        }
        let Some(param_ty) = paramref_owner
            .values()
            .find(|&&(owner, _)| owner == idx)
            .map(|&(_, ty)| ty)
        else {
            continue;
        };
        if idx >= site.args.len() {
            return false;
        }
        let (ty, size, align) = callee.param_home_shapes.get(idx).copied().unwrap_or((
            param_ty,
            ir_type_slot_size(param_ty),
            0,
        ));
        if callee
            .param_struct_sizes
            .get(idx)
            .copied()
            .flatten()
            .is_some_and(|n| n > size)
        {
            return false;
        }
        let dest = Value(next_fresh);
        let Some(next) = next_fresh.checked_add(1) else {
            return false;
        };
        next_fresh = next;
        synthesized.push((dest, ty, size, align));
        *home = Some((dest, ty, size));
    }
    let entry_block = &mut inlined_blocks[0];
    let prologue_end = entry_block
        .instructions
        .iter()
        .rposition(|inst| matches!(inst, Instruction::Alloca { .. }))
        .map_or(0, |i| i + 1);
    for (offset, (dest, ty, size, align)) in synthesized.into_iter().enumerate() {
        entry_block.insert_inst(
            prologue_end + offset,
            Instruction::Alloca {
                dest,
                ty,
                size,
                align,
                volatile: false,
                semantic_volatile: false,
            },
            crate::common::source::Span::dummy(),
        );
    }

    // ms178: SSA parameter substitution. For pure scalar params (home never
    // address-taken, never re-stored) a constant argument replaces every home
    // load, skipping the store+load round-trip; the home alloca is dropped.
    // Restricted to CONSTANT args: a runtime arg is an SSA value whose live
    // range crosses the call site, and LCCC's linear-scan allocator can leave
    // such a value in an undefined callee-saved register on some paths
    // (zlib-ng deflateSetParamPre *out=param wrote a stale %rbx). Constants
    // are materialized at the use site, so they have no register interaction.
    let orig_homes: Vec<Option<Value>> = param_alloca_info
        .iter()
        .map(|home| home.map(|(v, _, _)| Value(v.0 - value_offset)))
        .collect();
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

    // Parameter substitution's diagnostic switches, resolved once per
    // translation unit by `run_passes` rather than read here: the pass-pipeline
    // env-read ratchet (tests/regression/check_env_test_hygiene.sh) requires a
    // new switch to be resolved in `run_passes`, so no `getenv` runs per call
    // site.
    let SsaParamConfig {
        enabled: ssa_param_enabled,
        skip: ssa_skip,
        log: ssa_log,
    } = ssa_param_config();
    let substitutable: Vec<bool> = (0..param_alloca_info.len())
        .map(|i| {
            let ok = ssa_param_enabled
                && !ssa_skip.contains(&site.callee_name)
                && i < site.args.len()
                // ParamRef users are materialized below from their caller
                // store. Do not remove their home alloca through the older
                // load-only optimization.
                && !paramref_params.contains(&i)
                && orig_homes
                    .get(i)
                    .copied()
                    .flatten()
                    .is_some_and(|h| param_substitutable(callee, i, h));
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
            if let (Operand::Const(c), Some((home, _, _))) = (&site.args[i], param_alloca_info[i]) {
                if let Some(at) = const_ir_type(c) {
                    home_subst.insert(home.0, (site.args[i], at));
                }
            }
        }
    }

    // Insert stores/memcpys of arguments into the parameter homes. ParamRef
    // materializations follow the argument bridge so they are defined on every
    // inlined path before any cloned block can consume them.
    let mut paramref_next_value = next_fresh;
    let mut paramref_subst: FxInlineMap<u32, Operand> = FxInlineMap::default();
    let paramref_debug = PARAMREF_DEBUG_ENABLED.with(std::cell::Cell::get);
    {
        use crate::common::fx_hash::FxHashMap as ParamMap;
        let entry_block = &mut inlined_blocks[0];

        // A bridge store must follow the alloca it writes and precede the first
        // READ of that home, and those two constraints do not share a position:
        // a memory-passed parameter (`__int128`, `__float128`, a vector, a
        // struct) is read straight out of its home in the entry block, in the
        // middle of the prologue.  A single "after the allocas" position (first
        // run or last alloca) puts the store after that read -- a switch on an
        // `__int128` argument then read the uninitialized slot.  Each store
        // therefore goes immediately after ITS home's definition.
        let mut home_def_index: ParamMap<u32, usize> = ParamMap::default();
        for (index, inst) in entry_block.instructions.iter().enumerate() {
            if let Instruction::Alloca { dest, .. } = inst {
                home_def_index.insert(dest.0, index);
            }
        }
        let prologue_end = entry_block
            .instructions
            .iter()
            .rposition(|inst| matches!(inst, Instruction::Alloca { .. }))
            .map_or(0, |i| i + 1);
        // Every parameter with a live ParamRef retains this bridge even for a
        // constant argument: constants may replace the ParamRef directly, while
        // the home remains a valid ABI object for address-taken or later memory
        // uses.
        let has_spans = !entry_block.source_spans.is_empty();
        let num_args_to_store = std::cmp::min(site.args.len(), param_alloca_info.len());
        let mut bridge: Vec<(usize, Instruction)> = Vec::new();
        for i in 0..num_args_to_store {
            let Some((home, home_ty, _)) = param_alloca_info[i] else {
                continue;
            };
            if home_subst.contains_key(&home.0) {
                continue; // value flows via the older pure-home substitution
            }
            let pos = home_def_index
                .get(&home.0)
                .map_or(prologue_end, |index| index + 1);
            let param_struct_size = callee.param_struct_sizes.get(i).copied().flatten();
            if let Some(struct_size) = param_struct_size {
                // Struct-by-value parameter: the caller passes a pointer to the struct data.
                // We must copy the struct data from that pointer into the callee's param alloca.
                let Operand::Value(src_ptr) = site.args[i] else {
                    // Struct arg should always be a Value (pointer), not a Const.
                    return false;
                };
                bridge.push((
                    pos,
                    Instruction::Memcpy {
                        dest: home,
                        src: src_ptr,
                        size: struct_size,
                    },
                ));
            } else {
                let store_ty = home_ty;
                bridge.push((
                    pos,
                    Instruction::Store {
                        volatile: false,
                        val: site.args[i],
                        ptr: home,
                        ty: store_ty,
                        seg_override: AddressSpace::Default,
                    },
                ));
            }
        }
        let insert_high = bridge
            .iter()
            .map(|(pos, _)| *pos)
            .max()
            .unwrap_or(prologue_end);
        let inserted_arg_count = bridge.len();
        let mut placed: Vec<(u32, usize)> = Vec::with_capacity(bridge.len()); // (home, index)
        bridge.sort_by_key(|(pos, _)| *pos);
        for (offset, (pos, inst)) in bridge.into_iter().enumerate() {
            let at = pos + offset;
            if let Instruction::Store { ptr, .. } | Instruction::Memcpy { dest: ptr, .. } = &inst {
                placed.push((ptr.0, at));
            }
            entry_block.instructions.insert(at, inst);
            if has_spans {
                entry_block
                    .source_spans
                    .insert(at, crate::common::source::Span::dummy());
            }
        }

        // Ordering is a property of the FINAL instruction list, so it is checked
        // there: each bridge store must precede every read of its home, and the
        // materialization loads must precede the first use of the ParamRef they
        // replace.  A site that cannot have both declines the inline (returning
        // false discards the cloned body).
        for &(home, store_index) in &placed {
            for (index, inst) in entry_block.instructions.iter().enumerate() {
                if index >= store_index {
                    break;
                }
                if inst.used_values().contains(&home) {
                    return false;
                }
            }
        }

        let mut materialize_pos = insert_high + inserted_arg_count;
        for (paramref_dest, param_idx, _) in &paramref_records {
            if *param_idx >= num_args_to_store {
                continue;
            }
            let first_use = entry_block
                .instructions
                .iter()
                .position(|inst| inst.used_values().contains(&paramref_dest.0))
                .or_else(|| {
                    entry_block
                        .terminator
                        .used_values()
                        .contains(&paramref_dest.0)
                        .then_some(entry_block.instructions.len())
                });
            if first_use.is_some_and(|index| index < materialize_pos) {
                return false;
            }
        }
        // Deduplicate loads: one Load per distinct param_idx, and one Cast per
        // distinct (param_idx, param_ty) when types differ.  This reduces IR
        // bloat when a callee has multiple ParamRef for same parameter (common
        // after TCE / loop lowering) and keeps compile-time low.
        let mut loaded_per_param: ParamMap<usize, Value> = ParamMap::default();
        let mut cast_per_param_ty: ParamMap<(usize, IrType), Value> = ParamMap::default();

        for (paramref_dest, param_idx, param_ty) in &paramref_records {
            if *param_idx >= num_args_to_store || *param_idx >= param_alloca_info.len() {
                return false;
            }

            let Some((home, home_ty, _)) = param_alloca_info[*param_idx] else {
                return false;
            };

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
    let param_alloca_set: crate::common::fx_hash::FxHashSet<u32> = param_alloca_info
        .iter()
        .flatten()
        .map(|(v, _, _)| v.0)
        .collect();
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
    elide_readonly_local_memcpys(&mut inlined_blocks);
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
    //
    // LABEL PRESERVATION (Phase-0 fairness): when the split block's label is
    // PRIVATE — no other block branches to it and no LabelAddr exists — the
    // MERGE block keeps the original label and the pre-call remainder takes
    // the fresh one.  Every pre-existing call after the call site therefore
    // stays in an entry-labeled block, which is what makes the Phase-0
    // always_inline drain's entry-class test (label <= entry snapshot)
    // robust against chains of inlined splits: without this, a replicating
    // dispatcher inlined earlier in the same block would push surviving
    // contract sites into a fresh-labeled continuation and queue them
    // behind the churn it spawns (Review-AI finding 8, same-block
    // interleaving).  A visible external reference (loop back-edge, computed
    // goto) must keep landing in the PRE-CALL part, so the label stays
    // there and the merge block takes the fresh id — the historical shape.
    let call_block_idx = site.block_idx;
    let call_inst_idx = site.inst_idx;

    let split_label = caller.blocks[call_block_idx].label;
    // NOTE: the split block's own (original) terminator participates: a
    // self-loop `B: ...; goto B` whose body contains the call must NOT be
    // private — after the splice the original terminator lives in the
    // merge block, and keeping the label there would retarget the latch
    // edge to the merge itself, skipping the pre-call work and the whole
    // inlined body (sha256_transform segfault: its schedule loop is one
    // self-latching block around inlined helpers).
    let label_is_private = caller.blocks.iter().all(|b| match &b.terminator {
        Terminator::Branch(t) => *t != split_label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label != split_label && *false_label != split_label,
        Terminator::Switch { cases, default, .. } => {
            *default != split_label && !cases.iter().any(|(_, t)| *t == split_label)
        }
        _ => true,
    }) && !caller.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::LabelAddr { .. }))
    });

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

    // Label assignment per the privacy analysis above: private label =>
    // merge keeps it, pre-call remainder takes the fresh id; externally
    // referenced label => historical shape (merge takes the fresh id).
    let (merge_label, precall_label) = if label_is_private {
        (split_label, merge_block_id)
    } else {
        (merge_block_id, split_label)
    };
    caller.blocks[call_block_idx].label = precall_label;

    // The callee's return blocks were branched to the FRESH merge id; when
    // the merge kept the split block's label, those edges must follow the
    // merge's ACTUAL label or every return re-enters the pre-call block
    // (an infinite loop — the second half of the sha256_transform
    // segfault).
    if merge_label != merge_block_id {
        for block in inlined_blocks.iter_mut() {
            if matches!(block.terminator, Terminator::Branch(t) if t == merge_block_id) {
                block.terminator = Terminator::Branch(merge_label);
            }
        }
    }

    let merge_block = BasicBlock {
        label: merge_label,
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
    *global_max_block_id = (*global_max_block_id).max(merge_block_id.0);

    // The caller's label namespace must be advanced past every block we just
    // cloned. Without this, a later pass that allocates fresh block labels
    // from func.next_label (vectorizer, unroller, ...) reuses the inlined
    // blocks' labels, corrupting CFG edges — observed as an infinite
    // self-loop / out-of-bounds read in the lea_sib_fold -O2 regression
    // (exit CondBranch retargeted onto the loop header).
    let caller_max_label = caller
        .blocks
        .iter()
        .map(|block| block.label.0)
        .max()
        .unwrap_or(0);
    caller.next_label = caller
        .next_label
        .max(caller_max_label.saturating_add(1))
        .max(merge_block_id.0.saturating_add(1));

    // The cloned body keeps the stricter builtin contract of either source
    // context. This matters for GNU extern-inline implementations of libc
    // names: they have no out-of-line TU body, yet their copied loop must not
    // be re-synthesized into a call to the different external libc function.
    caller.pass_policy.builtins = caller
        .pass_policy
        .builtins
        .conservative_merge(callee.builtin_policy);
    caller.ssp.merge_inlined(callee.ssp);
    // The caller's body now holds the callee's ISA-dependent IR.
    let machine = crate::common::types::target_elf_machine();
    let caller_features = if matches!(machine, 3 | 62) {
        crate::backend::x86::isa::function_feature_bits(
            crate::common::x86_features::effective().raw(),
            crate::common::x86_features::explicit(),
            &caller.target_features,
            machine == 62,
        )
    } else {
        0
    };
    caller
        .x86_isa_needs
        .absorb(&callee.x86_isa_needs, caller_features);

    true
}

/// Remap a Value by adding an offset.
fn remap_value(v: Value, offset: u32) -> Value {
    Value(v.0 + offset)
}

/// Dense callee-label -> caller-label assignment for one inline clone.
///
/// One counter, one slot per distinct callee label: no arithmetic on label
/// *values*, so no wraparound and no exponential growth (see the comment at the
/// clone site for the miscompile this replaced).  A label the callee references
/// but does not define — a `possible_targets` entry or a stale phi predecessor —
/// still gets its own fresh label rather than aliasing a live block.
/// Label slots an inline needs ABOVE the counter's last minted label before
/// the label space is exhausted.
///
/// The dense counter hands out one label per distinct callee label, but the
/// clone also needs labels the callee never mentions:
///
/// * one for the merge block the inline branches to after the cloned body;
/// * up to three more minted on demand by [`LabelMap::get`] for label
///   references that no block defines — an asm `goto` target, or a terminator
///   that names a label reached only through an indirect path.
///
/// Reserving them at the base instead of discovering the exhaustion one
/// `u32::MAX` label later is what keeps the failure loud: an inline that
/// cannot mint the labels it needs refuses to clone (its caller keeps the
/// call), rather than minting a label that already exists.
const LABEL_MAP_RESERVED: u32 = 4;

struct LabelMap {
    next: u32,
    map: FxHashMap<u32, BlockId>,
}

impl LabelMap {
    fn new(base: u32) -> Self {
        Self {
            next: base,
            map: FxHashMap::default(),
        }
    }

    /// The caller label for a callee label, minting it if this is the first
    /// reference (references always precede or accompany the definition in the
    /// remap walk because every block's label is assigned up front).
    fn get(&mut self, label: BlockId) -> BlockId {
        if let Some(mapped) = self.map.get(&label.0) {
            return *mapped;
        }
        self.assign(label)
    }

    /// Assign a label to a callee label that has none yet.
    fn assign(&mut self, label: BlockId) -> BlockId {
        let mapped = self.assign_fresh();
        self.map.insert(label.0, mapped);
        mapped
    }

    /// A brand-new label, which is above every label the clone will use.
    ///
    /// The capacity is pre-checked at the base (see [`LABEL_MAP_RESERVED`]):
    /// `base + blocks + reserved <= u32::MAX` is a precondition of every mint in
    /// this map. The assert turns that precondition into a loud failure instead
    /// of a bounds test: a release build that wrapped here would mint a label
    /// that already exists, which is a miscompile, so aborting is the
    /// least-bad outcome if the reservation is ever reduced below what a clone
    /// actually mints.
    fn assign_fresh(&mut self) -> BlockId {
        let id = BlockId(self.next);
        assert!(
            self.next < u32::MAX,
            "inline label capacity was pre-checked"
        );
        self.next += 1;
        id
    }
}

/// Remap a BlockId through the clone's dense label assignment.
fn remap_block(b: BlockId, labels: &mut LabelMap) -> BlockId {
    labels.get(b)
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
        calling_convention: info.calling_convention,
        is_fastcall: info.is_fastcall,
        regparm: info.regparm,
        is_pure: info.is_pure,
        is_const: info.is_const,
        // A remapped call is the same call: it still reaches (or not) the
        // library function.
        assume_library: info.assume_library,
        ret_eightbyte_classes: info.ret_eightbyte_classes.clone(),
        ret_is_f128_sse: info.ret_is_f128_sse,
        cold: info.cold,
    }
}

/// Remap all values and block references in an instruction.
fn remap_instruction(inst: &Instruction, vo: u32, labels: &mut LabelMap) -> Instruction {
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
            eightbyte_classes,
        } => Instruction::VaArgStruct {
            dest_ptr: remap_value(*dest_ptr, vo),
            va_list_ptr: remap_value(*va_list_ptr, vo),
            size: *size,
            align: *align,
            eightbyte_classes: eightbyte_classes.clone(),
        },
        Instruction::VaArgVector {
            dest_ptr,
            va_list_ptr,
            size,
            align,
            eightbyte_classes,
        } => Instruction::VaArgVector {
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
                .map(|(op, bid)| (remap_operand(op, vo), remap_block(*bid, labels)))
                .collect(),
        },
        Instruction::LabelAddr { dest, label } => Instruction::LabelAddr {
            dest: remap_value(*dest, vo),
            // A label address is local to the cloned callee body just like a
            // branch target. Keeping the original BlockId made every inline
            // clone point at another function/clone's label (or an undefined
            // .LBB symbol), violating GCC's per-clone label identity.
            label: remap_block(*label, labels),
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
                .map(|(name, bid)| (name.clone(), remap_block(*bid, labels)))
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
fn remap_terminator(term: &Terminator, vo: u32, labels: &mut LabelMap) -> Terminator {
    match term {
        Terminator::Return(op) => Terminator::Return(op.map(|o| remap_operand(&o, vo))),
        Terminator::Branch(bid) => Terminator::Branch(remap_block(*bid, labels)),
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => Terminator::CondBranch {
            cond: remap_operand(cond, vo),
            true_label: remap_block(*true_label, labels),
            false_label: remap_block(*false_label, labels),
        },
        Terminator::IndirectBranch {
            target,
            possible_targets,
        } => Terminator::IndirectBranch {
            target: remap_operand(target, vo),
            possible_targets: possible_targets
                .iter()
                .map(|b| remap_block(*b, labels))
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
                .map(|&(v, bid)| (v, remap_block(bid, labels)))
                .collect(),
            default: remap_block(*default, labels),
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
        // Three sites are refused at every level (see
        // MAX_STATIC_LOOP_INLINE_CALLS): the find_next_andnot_bit shape
        // measures a tie at -O2 and a loss at -O3 when inlined.
        assert!(!fits_static_loop_inline_limits(60, 13, 3));
        assert!(!fits_static_loop_inline_limits(41, 9, 4));
    }
    #[test]
    fn aggregate_frame_veto_is_size_optimized_and_multisite_only() {
        // The arch/x86/boot `struct biosregs` shape: a ~15-instruction helper
        // that still forces 44 frame bytes on every inline site.
        let mut callee = wrapper_policy_data(2);
        callee.agg_frame_bytes = 44;
        assert!(aggregate_frame_should_stay_outlined(&callee, false, true));

        // -O1/-O2/-O3 optimise for speed; stack cost alone must not veto.
        assert!(!aggregate_frame_should_stay_outlined(&callee, false, false));

        // One call site: the outlined body disappears, so inlining is
        // size-neutral and the veto must stay out of the way.
        callee.single_call_site = true;
        assert!(!aggregate_frame_should_stay_outlined(&callee, false, true));
        callee.single_call_site = false;

        // Callees that have no usable out-of-line body must still be inlined.
        callee.is_always_inline = true;
        assert!(!aggregate_frame_should_stay_outlined(&callee, false, true));
        callee.is_always_inline = false;
        callee.is_gnu_inline_def = true;
        assert!(!aggregate_frame_should_stay_outlined(&callee, false, true));
        callee.is_gnu_inline_def = false;

        // A profile-forced hot site outranks the static size heuristic.
        assert!(!aggregate_frame_should_stay_outlined(&callee, true, true));

        // Below the threshold the allocas are promotable, so no frame cost.
        callee.agg_frame_bytes = AGG_FRAME_SLOT_BYTES - 1;
        assert!(!aggregate_frame_should_stay_outlined(&callee, false, true));
        // At exactly the threshold the veto engages.
        callee.agg_frame_bytes = AGG_FRAME_SLOT_BYTES;
        assert!(aggregate_frame_should_stay_outlined(&callee, false, true));
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
            target_features: Vec::new(),
            builtin_policy: Default::default(),
            ssp: Default::default(),
            blocks: (0..block_count)
                .map(|index| BasicBlock {
                    label: BlockId(index as u32),
                    instructions: Vec::new(),
                    terminator: Terminator::Return(None),
                    source_spans: Vec::new(),
                })
                .collect(),
            param_struct_sizes: Vec::new(),
            param_alloca_values: Vec::new(),
            param_home_shapes: Vec::new(),
            return_type: IrType::Void,
            num_params: 0,
            next_value_id: 0,
            max_block_id: 0,
            single_call_site: false,
            agg_frame_bytes: 0,
            is_always_inline: false,
            x86_isa_needs: Default::default(),
            uses_fp_or_simd: false,
            exceeds_normal_limits: false,
            is_static_inline: false,
            is_gnu_inline_def: false,
            is_plain_static: true,
            direct_call_count: 2,
            loop_bound_params: 0,
            const_trip_only: false,
            const_branch_params: 0,
            const_branch_only: false,
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

    /// A declared-inline looping callee of `blocks` blocks and
    /// `instructions` instructions in total (spread over the blocks).
    fn loop_helper_data(blocks: usize, instructions: usize) -> CalleeData {
        let mut data = wrapper_policy_data(blocks);
        let copy = Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I32(0)),
        };
        for i in 0..instructions {
            data.blocks[i % blocks].instructions.push(copy.clone());
        }
        data.is_plain_static = false;
        data.is_static_inline = true;
        data.has_loops = true;
        data
    }

    #[test]
    fn hot_loop_helper_is_a_bounded_declared_inline_loop() {
        // zstd's ZSTD_count after its own helpers: 142 instructions, 19 blocks.
        assert!(is_hot_loop_helper_candidate(&loop_helper_data(19, 142)));
        let limit = loop_helper_data(MAX_HOT_LOOP_HELPER_BLOCKS, MAX_HOT_LOOP_HELPER_INSTRUCTIONS);
        assert!(is_hot_loop_helper_candidate(&limit));
        let wide = MAX_HOT_LOOP_HELPER_INSTRUCTIONS + 1;
        assert!(!is_hot_loop_helper_candidate(&loop_helper_data(19, wide)));
        let branchy = MAX_HOT_LOOP_HELPER_BLOCKS + 1;
        assert!(!is_hot_loop_helper_candidate(&loop_helper_data(
            branchy, 142
        )));
        let reject = |edit: fn(&mut CalleeData)| {
            let mut data = loop_helper_data(19, 142);
            edit(&mut data);
            !is_hot_loop_helper_candidate(&data)
        };
        // Loop-free bodies are the hot-leaf rule's; plain statics keep the
        // historical caps; always_inline, recursion and constant-trip clones
        // have their own policies.
        assert!(reject(|d| d.has_loops = false));
        assert!(reject(|d| {
            d.is_static_inline = false;
            d.is_plain_static = true;
        }));
        assert!(reject(|d| d.is_always_inline = true));
        assert!(reject(|d| d.is_recursive = true));
        assert!(reject(|d| d.const_trip_only = true));
    }

    #[test]
    fn large_mandatory_template_wrapper_is_deferred_not_permanently_banned() {
        let mut data = wrapper_policy_data(1);
        data.single_call_site = true;
        data.is_single_call_site_static = true;
        data.direct_call_count = 1;
        data.has_inlineable_loop_descendant = true;
        data.size_inline_cost = MAX_INLINE_BUDGET_PER_CALLER + 1;
        let refused = |d: &CalleeData, raw, section, pgo, size| {
            template_wrapper_should_wait_for_specialization(d, raw, section, pgo, size)
        };
        assert!(refused(&data, 21, false, false, false));
        assert!(!refused(&data, 21, true, false, false));
        assert!(!refused(&data, 21, false, true, false));
        assert!(!refused(&data, 21, false, false, true));
        assert!(!refused(
            &data,
            MAX_SMALL_STATIC_LOOP_INLINE_INSTRUCTIONS + 1,
            false,
            false,
            false
        ));
        data.size_inline_cost = MAX_INLINE_BUDGET_PER_CALLER;
        assert!(!refused(&data, 21, false, false, false));
        data.size_inline_cost += 1;
        data.is_plain_static = false;
        assert!(!refused(&data, 21, false, false, false));
        data.is_plain_static = true;
        data.has_inlineable_loop_descendant = false;
        assert!(!refused(&data, 21, false, false, false));
        data.has_inlineable_loop_descendant = true;
        // Once expansion/specialization exposes the real loop, normal inline
        // policy can consider it; do not blacklist the function forever.
        data.has_loops = true;
        data.has_inlined_calls = true;
        assert!(!refused(&data, 21, false, false, false));
        data.has_loops = false;
        data.blocks = wrapper_policy_data(MAX_SMALL_STATIC_LOOP_INLINE_BLOCKS + 1).blocks;
        assert!(!refused(&data, 21, false, false, false));
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
            pass_policy: Default::default(),
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
            x86_isa_needs: Default::default(),
            is_noinline: false,
            target_features: Vec::new(),
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
            calling_convention: crate::common::types::CallingConvention::Default,
            is_fastcall: false,
            regparm: None,
            is_naked: false,
            no_instrument: false,
            global_init_label_blocks: vec![],
            ret_eightbyte_classes: vec![],
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
            ssp: Default::default(),
            body_span: crate::common::source::Span::dummy(),
            terminator_spans: Default::default(),
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
            is_retain: false,
            is_nocommon: false,
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

#[cfg(test)]
mod phase0_drain_tests {
    use super::*;
    use crate::common::types::EightbyteClass;
    use crate::ir::reexports::{
        BasicBlock, BlockId, CallInfo, Instruction, IrBinOp, IrConst, IrFunction, IrModule,
        IrParam, Operand, Terminator, Value,
    };

    fn param0() -> Operand {
        Operand::Value(Value(0))
    }

    fn void_call(callee: &str, arg: Operand) -> Instruction {
        Instruction::Call {
            func: callee.to_string(),
            info: CallInfo {
                args: vec![arg],
                arg_types: vec![IrType::I32],
                return_type: IrType::Void,
                num_fixed_args: 1,
                ..crate::ir::reexports::CallInfo::default()
            },
        }
    }

    /// The scheduler-starvation shape behind kernel integrity_audit.o's
    /// surviving `fortify_memset_chk` call (F12): an always_inline callee
    /// that REPLICATES its own call sites through macro dispatchers (each
    /// inlined clone re-entering the same dispatcher — modeled by two
    /// self-calls, net +1 site per inline) sits in front of a plain
    /// always_inline callee in block order.  The economics passes inline
    /// ONE site per round under a 200-round cap, so the replicator burns
    /// every round and the plain callee's contract call survives — a hard
    /// undefined-symbol link error for `extern __gnu_inline__` wrappers.
    /// Phase 0 must drain the entry sites BEFORE the churn can matter.
    #[test]
    fn phase0_drains_contract_sites_despite_replicating_dispatchers() {
        let mut replicator = IrFunction::new("replicator".to_string(), IrType::Void, vec![], false);
        replicator.is_static = true;
        replicator.is_always_inline = true;
        replicator.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: param0(),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                // Two self-calls: every inlining of this body leaves ONE
                // more replicator site behind than it consumed.
                void_call("replicator", Operand::Value(Value(1))),
                void_call("replicator", Operand::Value(Value(1))),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let _ = replicator.next_value_id;

        let mut fortify_like =
            IrFunction::new("fortify_like".to_string(), IrType::I32, vec![], false);
        fortify_like.is_static = true;
        fortify_like.is_always_inline = true;
        // 8 adds: above the tiny threshold, so the old selector never
        // reached it once the replicator's tiny sites monopolized rounds.
        let mut insts = Vec::new();
        for i in 1..=8u32 {
            insts.push(Instruction::BinOp {
                dest: Value(i),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(i - 1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            });
        }
        fortify_like.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(Operand::Value(Value(8)))),
            source_spans: Vec::new(),
        });

        let mut caller = IrFunction::new("f".to_string(), IrType::Void, vec![], false);
        caller.next_value_id = 10;
        caller.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(7)),
                },
                void_call("replicator", param0()),
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        caller.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Call {
                func: "fortify_like".to_string(),
                info: CallInfo {
                    dest: Some(Value(9)),
                    args: vec![param0()],
                    arg_types: vec![IrType::I32],
                    return_type: IrType::I32,
                    num_fixed_args: 1,
                    ..crate::ir::reexports::CallInfo::default()
                },
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let mut module = IrModule::new();
        module.functions = vec![caller, replicator, fortify_like];

        super::inline_run(&mut module, false);

        let caller = &module.functions[0];
        let survives = caller.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func, .. } if func == "fortify_like"))
        });
        assert!(
            !survives,
            "always_inline contract site survived the inliner: \
             Phase 0 must drain entry sites before clone churn"
        );
    }

    /// SAME-BLOCK interleaving (Review-AI finding 8): when the replicating
    /// dispatcher drains FIRST, the split historically moved the surviving
    /// contract call (later in the SAME block) into a fresh-labeled merge
    /// block, demoting it to clone-born class — the churn then starved it
    /// under the shared budget.  inline_call_site now lets the merge block
    /// KEEP the split block's label whenever that label is private, so the
    /// contract site stays entry-class for the whole drain.
    #[test]
    fn phase0_survives_contract_site_after_same_block_dispatcher_split() {
        let mut replicator = IrFunction::new("replicator".to_string(), IrType::Void, vec![], false);
        replicator.is_static = true;
        replicator.is_always_inline = true;
        replicator.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                void_call("replicator", param0()),
                void_call("replicator", param0()),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let mut fortify_like =
            IrFunction::new("fortify_like".to_string(), IrType::I32, vec![], false);
        fortify_like.is_static = true;
        fortify_like.is_always_inline = true;
        let mut insts = Vec::new();
        for i in 1..40 {
            insts.push(Instruction::BinOp {
                dest: Value(i),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(i - 1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            });
        }
        fortify_like.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(Operand::Value(Value(8)))),
            source_spans: Vec::new(),
        });

        let mut caller = IrFunction::new("f".to_string(), IrType::Void, vec![], false);
        caller.next_value_id = 10;
        caller.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                void_call("replicator", param0()),
                Instruction::Call {
                    func: "fortify_like".to_string(),
                    info: CallInfo {
                        dest: Some(Value(9)),
                        args: vec![param0()],
                        arg_types: vec![IrType::I32],
                        return_type: IrType::I32,
                        num_fixed_args: 1,
                        ..crate::ir::reexports::CallInfo::default()
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let mut module = IrModule::new();
        module.functions = vec![caller, replicator, fortify_like];

        super::inline_run(&mut module, false);

        let caller = &module.functions[0];
        let survives = caller.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func, .. } if func == "fortify_like"))
        });
        assert!(
            !survives,
            "contract site after a same-block replicator split was starved: \
             the merge block must keep the split block's label (entry class)"
        );
    }

    /// LABEL-PRESERVING SPLIT — structural invariants (the sha256_transform
    /// segfault class).  When a private-label block is split by an inline:
    ///   1. the merge block KEEPS the split block's label (the Phase-0
    ///      entry-class test stays sound and external flow is preserved);
    ///   2. every cloned return block branches to the merge block's ACTUAL
    ///      label — the return edges are wired to the fresh id at build
    ///      time and must be re-pointed, or returns re-enter the pre-call
    ///      block and the caller loops forever.
    ///   3. no block may branch to the pre-call block's fresh label except
    ///      the pre-call -> clone-entry edge itself.
    #[test]
    fn label_preserving_split_keeps_label_and_retargets_returns() {
        // Callee with TWO return paths => cloned return blocks with
        // Branch(merge_block_id) edges.
        let mut two_way = IrFunction::new("two_way".to_string(), IrType::I32, vec![], false);
        two_way.is_static = true;
        two_way.is_always_inline = true;
        two_way.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        for lbl in [1u32, 2u32] {
            two_way.blocks.push(BasicBlock {
                label: BlockId(lbl),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(7)))),
                source_spans: Vec::new(),
            });
        }

        // Caller: ONE block, no branches at all => label 0 is private.
        let mut caller = IrFunction::new("f".to_string(), IrType::I32, vec![], false);
        caller.next_value_id = 10;
        caller.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::Call {
                    func: "two_way".to_string(),
                    info: CallInfo {
                        dest: Some(Value(9)),
                        args: vec![param0()],
                        arg_types: vec![IrType::I32],
                        return_type: IrType::I32,
                        num_fixed_args: 1,
                        ..crate::ir::reexports::CallInfo::default()
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(9)))),
            source_spans: Vec::new(),
        });

        let mut module = IrModule::new();
        module.functions = vec![caller, two_way];

        super::inline_run(&mut module, false);

        let caller = &module.functions[0];
        // (1) the original label survives on the block carrying the
        // original terminator.
        let merge = caller
            .blocks
            .iter()
            .find(|b| {
                matches!(
                    b.terminator,
                    Terminator::Return(Some(Operand::Value(Value(9))))
                )
            })
            .expect("merge block with original terminator");
        assert_eq!(
            merge.label,
            BlockId(0),
            "private split label must stay on the merge block, blocks: {:?}",
            caller.blocks.iter().map(|b| b.label).collect::<Vec<_>>()
        );
        // (2) both cloned return blocks branch to that label.
        let ret_targets: Vec<BlockId> = caller
            .blocks
            .iter()
            .filter_map(|b| match b.terminator {
                Terminator::Branch(t) => Some(t),
                _ => None,
            })
            .collect();
        // 1 pre-call -> clone-entry edge + 2 cloned return edges.
        assert_eq!(
            ret_targets.len(),
            3,
            "pre-call edge plus two cloned return edges expected, got {ret_targets:?}"
        );
        let to_merge = ret_targets.iter().filter(|t| **t == BlockId(0)).count();
        assert_eq!(
            to_merge, 2,
            "both cloned return edges must target the merge label (BlockId 0), got {ret_targets:?}"
        );
        // (3) the fresh pre-call label (whatever it is) receives edges only
        // from the pre-call block itself: no block OTHER than pre-call
        // branches to it.  Pre-call's own edge targets the clone entry, so
        // the fresh label must appear as a branch target ZERO times.
        let precall_fresh = caller.blocks[0].label;
        assert_ne!(
            precall_fresh,
            BlockId(0),
            "pre-call block must carry the fresh label"
        );
        let fresh_inbound = caller
            .blocks
            .iter()
            .filter(|b| matches!(b.terminator, Terminator::Branch(t) if t == precall_fresh))
            .count();
        assert_eq!(
            fresh_inbound, 0,
            "nothing may branch to the pre-call block's fresh label"
        );
    }
}

#[cfg(test)]
mod const_trip_tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::ops::IrCmpOp;
    use crate::ir::reexports::{BasicBlock, Instruction, Terminator};

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn function(blocks: Vec<BasicBlock>) -> IrFunction {
        IrFunction {
            pass_policy: Default::default(),
            name: "callee".into(),
            return_type: IrType::Void,
            params: vec![],
            blocks,
            is_variadic: false,
            is_declaration: false,
            is_static: true,
            is_inline: false,
            is_always_inline: false,
            x86_isa_needs: Default::default(),
            is_noinline: false,
            target_features: Vec::new(),
            next_value_id: 16,
            fp_expr_tags: Default::default(),
            next_label: 8,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: vec![],
            uses_sret: false,
            calling_convention: crate::common::types::CallingConvention::Default,
            is_fastcall: false,
            regparm: None,
            is_naked: false,
            no_instrument: false,
            global_init_label_blocks: vec![],
            ret_eightbyte_classes: vec![],
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
            ssp: Default::default(),
            body_span: crate::common::source::Span::dummy(),
            terminator_spans: Default::default(),
        }
    }

    fn param_ref(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::I32,
        }
    }

    /// `i < bound` with a constant lhs, so only `bound` can be a parameter.
    fn compare_const_lt(dest: u32, bound: Value) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op: IrCmpOp::Slt,
            lhs: Operand::Const(IrConst::I32(0)),
            rhs: Operand::Value(bound),
            ty: IrType::I32,
        }
    }

    /// entry(0) reads the parameter, header(1) exits to block 3 on
    /// `0 < bound`, and the body(2) branches back to the header.
    fn counted_loop(header_bound: Value, entry: Vec<Instruction>) -> IrFunction {
        function(vec![
            block(0, entry, Terminator::Branch(BlockId(1))),
            block(
                1,
                vec![compare_const_lt(3, header_bound)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(2, vec![], Terminator::Branch(BlockId(1))),
            block(3, vec![], Terminator::Return(None)),
        ])
    }

    #[test]
    fn mask_marks_parameter_bounding_a_loop_exit() {
        let f = counted_loop(Value(1), vec![param_ref(1, 1)]);
        assert_eq!(loop_bound_param_mask(&f), 0b10);
    }

    #[test]
    fn mask_follows_copies_to_the_parameter() {
        let f = counted_loop(
            Value(5),
            vec![
                param_ref(1, 1),
                Instruction::Copy {
                    dest: Value(4),
                    src: Operand::Value(Value(1)),
                },
                Instruction::Copy {
                    dest: Value(5),
                    src: Operand::Value(Value(4)),
                },
            ],
        );
        assert_eq!(loop_bound_param_mask(&f), 0b10);
    }

    #[test]
    fn mask_ignores_comparisons_outside_any_loop() {
        let f = function(vec![
            block(
                0,
                vec![param_ref(1, 0), compare_const_lt(2, Value(1))],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(None)),
            block(2, vec![], Terminator::Return(None)),
        ]);
        assert_eq!(loop_bound_param_mask(&f), 0);
    }

    #[test]
    fn mask_ignores_loops_without_an_exit_edge() {
        // header(1) branches to block 2 or back to itself; both successors
        // stay inside the loop body, so no compare is a loop exit.
        let f = function(vec![
            block(0, vec![param_ref(1, 1)], Terminator::Branch(BlockId(1))),
            block(
                1,
                vec![compare_const_lt(3, Value(1))],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(2),
                    false_label: BlockId(2),
                },
            ),
            block(2, vec![], Terminator::Branch(BlockId(1))),
        ]);
        assert_eq!(loop_bound_param_mask(&f), 0);
    }

    #[test]
    fn mask_ignores_bounds_that_are_not_a_parameter() {
        // The bound is an undefined value here, standing in for a phi or an
        // arithmetic result. Such a bound is not proven to be the parameter.
        let f = counted_loop(Value(9), vec![param_ref(1, 1)]);
        assert_eq!(loop_bound_param_mask(&f), 0);
    }

    fn c32(v: i32) -> Operand {
        Operand::Const(IrConst::I32(v))
    }

    #[test]
    fn site_accepts_constant_trip_counts_from_two_to_eight() {
        let sizes = [None, None];
        for v in 2..=8 {
            assert!(const_trip_site_arg(
                0b10,
                &sizes,
                &[Operand::Value(Value(1)), c32(v)]
            ));
        }
    }

    #[test]
    fn site_rejects_trip_counts_outside_the_range() {
        let sizes = [None, None];
        for v in [-1, 0, 1, 9, 4096] {
            assert!(
                !const_trip_site_arg(0b10, &sizes, &[Operand::Value(Value(1)), c32(v)]),
                "trip {v}"
            );
        }
    }

    #[test]
    fn site_rejects_non_constant_and_unmarked_arguments() {
        let sizes = [None, None];
        // A value argument is never a known constant, even if it is one in practice.
        assert!(!const_trip_site_arg(
            0b10,
            &sizes,
            &[Operand::Value(Value(1)), Operand::Value(Value(2))]
        ));
        // The constant is at index 0, but only index 1 bounds a loop.
        assert!(!const_trip_site_arg(
            0b10,
            &sizes,
            &[c32(4), Operand::Value(Value(2))]
        ));
        // No loop-bound parameter at all.
        assert!(!const_trip_site_arg(
            0,
            &sizes,
            &[Operand::Value(Value(1)), c32(4)]
        ));
    }

    #[test]
    fn site_rejects_struct_by_value_bound_parameter() {
        // A struct-by-value parameter is never passed as a scalar constant.
        let sizes = [None, Some(8)];
        assert!(!const_trip_site_arg(
            0b10,
            &sizes,
            &[Operand::Value(Value(1)), c32(4)]
        ));
    }

    #[test]
    fn site_accepts_wide_integer_constants_in_range() {
        let sizes = [None, None];
        let arg = Operand::Const(IrConst::I64(4));
        assert!(const_trip_site_arg(
            0b10,
            &sizes,
            &[Operand::Value(Value(1)), arg]
        ));
    }
}

#[cfg(test)]
mod devirtualization_tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::{IrModule, IrParam};
    use crate::ir::reexports::{BasicBlock, CallInfo, Instruction, Terminator};

    fn scalar(ty: IrType) -> IrParam {
        IrParam {
            ty,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }
    }

    /// static int leaf(int x) { return x + 1; }
    fn leaf() -> IrFunction {
        let mut f = IrFunction::new("leaf".into(), IrType::I32, vec![scalar(IrType::I32)], false);
        f.is_static = true;
        f.next_value_id = 3;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                // Frontend shape: a parameter home precedes its ParamRef.
                Instruction::Alloca {
                    dest: Value(2),
                    ty: IrType::I32,
                    size: 4,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(0)),
                    ptr: Value(2),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });
        f
    }

    fn indirect(ptr: Value, arg_type: IrType) -> Instruction {
        Instruction::CallIndirect {
            func_ptr: Operand::Value(ptr),
            info: CallInfo {
                dest: Some(Value(4)),
                args: vec![Operand::Const(IrConst::I32(41))],
                arg_types: vec![arg_type],
                return_type: IrType::I32,
                num_fixed_args: 1,
                struct_arg_sizes: vec![None],
                struct_arg_aligns: vec![None],
                struct_arg_classes: vec![Vec::new()],
                struct_arg_riscv_float_classes: vec![None],
                struct_arg_is_f128_sse: vec![false],
                ..CallInfo::default()
            },
        }
    }

    fn module(target: &str, extra: Vec<Instruction>, arg_type: IrType) -> IrModule {
        let mut caller = IrFunction::new("caller".into(), IrType::I32, vec![], false);
        caller.next_value_id = 6;
        let mut instructions = vec![Instruction::GlobalAddr {
            dest: Value(2),
            name: target.into(),
        }];
        instructions.extend(extra);
        instructions.push(Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(2)),
        });
        instructions.push(indirect(Value(3), arg_type));
        caller.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        });
        let mut module = IrModule::default();
        module.functions = vec![leaf(), caller];
        module
    }

    fn calls(module: &IrModule) -> (usize, usize) {
        let mut direct = 0;
        let mut indirect = 0;
        for inst in module.functions[1]
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
        {
            match inst {
                Instruction::Call { func, .. } if func == "leaf" => direct += 1,
                Instruction::CallIndirect { .. } => indirect += 1,
                _ => {}
            }
        }
        (direct, indirect)
    }

    #[test]
    fn unique_constant_target_becomes_direct_and_inlines() {
        let mut m = module("leaf", Vec::new(), IrType::I32);
        assert_eq!(
            devirtualize_constant_indirect_calls(&mut m),
            [("caller".to_string(), "leaf".to_string())]
                .into_iter()
                .collect()
        );
        assert_eq!(calls(&m), (1, 0));
        let mut m = module("leaf", Vec::new(), IrType::I32);
        assert!(run(&mut m) >= 1);
        assert_eq!(calls(&m), (0, 0), "devirtualized tiny leaf must be inlined");
    }

    #[test]
    fn reassigned_pointer_is_not_a_constant() {
        let reassigned = vec![Instruction::Copy {
            dest: Value(2),
            src: Operand::Const(IrConst::I64(0)),
        }];
        let mut m = module("leaf", reassigned, IrType::I32);
        assert!(devirtualize_constant_indirect_calls(&mut m).is_empty());
        assert_eq!(calls(&m), (0, 1));
    }

    #[test]
    fn abi_mismatch_and_ineligible_targets_stay_indirect() {
        let mut m = module("leaf", Vec::new(), IrType::I64);
        assert!(
            devirtualize_constant_indirect_calls(&mut m).is_empty(),
            "argument type mismatch"
        );
        for edit in 0..4 {
            let mut m = module("leaf", Vec::new(), IrType::I32);
            match edit {
                0 => m.functions[0].is_weak = true,
                1 => m.functions[0].is_variadic = true,
                2 => {
                    m.asm_labels.insert("leaf".into(), "leaf_impl".into());
                }
                _ => {
                    m.functions[0].is_declaration = true;
                    m.functions[0].blocks.clear();
                }
            }
            assert!(
                devirtualize_constant_indirect_calls(&mut m).is_empty(),
                "edit {edit}"
            );
            assert_eq!(calls(&m), (0, 1), "edit {edit}");
        }
        let mut m = module("object", Vec::new(), IrType::I32);
        assert!(
            devirtualize_constant_indirect_calls(&mut m).is_empty(),
            "data address"
        );
    }

    #[test]
    fn every_iteration_blocks_exclude_conditional_loop_paths() {
        // 0 -> 1(header) -> {2, 6(exit)}; 2 -> {3, 4}; 3 -> 5; 4 -> 5; 5 -> 1.
        let cond = |t: u32, f: u32| Terminator::CondBranch {
            cond: Operand::Const(IrConst::I32(1)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        };
        let mut f = IrFunction::new("loop".into(), IrType::Void, vec![], false);
        for (label, term) in [
            (0, Terminator::Branch(BlockId(1))),
            (1, cond(2, 6)),
            (2, cond(3, 4)),
            (3, Terminator::Branch(BlockId(5))),
            (4, Terminator::Branch(BlockId(5))),
            (5, Terminator::Branch(BlockId(1))),
            (6, Terminator::Return(None)),
        ] {
            f.blocks.push(BasicBlock {
                label: BlockId(label),
                instructions: Vec::new(),
                terminator: term,
                source_spans: Vec::new(),
            });
        }
        let cfg = crate::ir::analysis::CfgAnalysis::build(&f);
        let loops =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        assert_eq!(loops.len(), 1);
        let mut every = every_iteration_blocks_of(&cfg, &loops[0]);
        every.sort_unstable();
        assert_eq!(
            every,
            vec![1, 2, 5],
            "conditional arms 3/4 are not per-iteration"
        );
    }
}

#[cfg(test)]
mod feature_switch_tests {
    //! Each inliner extension switch must change the result on the shape it
    //! exists for; otherwise the switch (and the extension) is untested.
    use super::*;
    use crate::ir::reexports::{
        BasicBlock, BlockId, CallInfo, Instruction, IrBinOp, IrConst, IrFunction, IrModule,
        Operand, Terminator, Value,
    };

    fn call(callee: &str) -> Instruction {
        Instruction::Call {
            func: callee.to_string(),
            info: CallInfo {
                args: vec![Operand::Const(IrConst::I32(0))],
                arg_types: vec![IrType::I32],
                return_type: IrType::Void,
                num_fixed_args: 1,
                ..CallInfo::default()
            },
        }
    }

    /// A one-block void function of `work` instructions (0, or >= 3), then
    /// `calls`. The work is live and irreducible -- load a global, square it
    /// `work - 3` times, store it back -- so the snapshot cleanup cannot fold
    /// it away (a `+1` chain would reassociate into one add, silently
    /// changing every size class these policy tests are built on).
    fn function(name: &str, work: u32, calls: &[&str]) -> IrFunction {
        assert!(
            work == 0 || work >= 3,
            "global address, load and store are the minimum work"
        );
        let mut f = IrFunction::new(name.to_string(), IrType::Void, vec![], false);
        let mut instructions = Vec::new();
        let mut next_value = 1;
        if work >= 3 {
            let last = work - 1;
            instructions.push(Instruction::GlobalAddr {
                dest: Value(1),
                name: "sink".to_string(),
            });
            instructions.push(Instruction::Load {
                dest: Value(2),
                ptr: Value(1),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            instructions.extend((3..=last).map(|i| Instruction::BinOp {
                dest: Value(i),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(i - 1)),
                rhs: Operand::Value(Value(i - 1)),
                ty: IrType::I32,
            }));
            instructions.push(Instruction::Store {
                val: Operand::Value(Value(last)),
                ptr: Value(1),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            next_value = last + 1;
        }
        instructions.extend(calls.iter().map(|c| call(c)));
        f.next_value_id = next_value;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        f
    }

    fn calls_to(module: &IrModule, caller: &str, callee: &str) -> usize {
        module
            .functions
            .iter()
            .filter(|f| f.name == caller)
            .flat_map(|f| f.blocks.iter())
            .flat_map(|b| b.instructions.iter())
            .filter(|i| matches!(i, Instruction::Call { func, .. } if func == callee))
            .count()
    }

    fn run(mut module: IrModule, features: InlineFeatures) -> IrModule {
        inline_run_with(&mut module, false, false, features);
        module
    }

    /// Second-pass always_inline templates expose ordinary calls. `gen`
    /// holds more mandatory `ai` sites than Phase 0's work counter and the
    /// first pass's always_inline budget admit, so the last `ai` clones are
    /// inserted by the second pass, after the ordinary rounds ended; each
    /// carries a call to the tiny `leaf`, which the ordinary policy accepts
    /// at any caller size. Phase 0 meters only callees on a call cycle, so
    /// `ai` recurses through the never-inlined `rec`: acyclic templates are
    /// drained completely and would never reach the second pass. `f` comes
    /// first in module (processing) order, so it clones the raw `gen` rather
    /// than `gen`'s processed snapshot, in which `gen`'s own drain left no
    /// `ai` site.
    fn joint_fixpoint_module() -> IrModule {
        let mut leaf = function("leaf", 3, &[]);
        leaf.is_static = true;
        leaf.is_inline = true;
        let mut ai = function("ai", 24, &["leaf", "rec"]);
        ai.is_static = true;
        ai.is_always_inline = true;
        let mut rec = function("rec", 0, &["ai"]);
        rec.is_noinline = true;
        let mut generator = function("gen", 0, &["ai"; 40]);
        generator.is_static = true;
        generator.is_always_inline = true;
        let caller = function("f", 0, &["gen"]);
        let mut module = IrModule::new();
        module.functions = vec![caller, leaf, ai, rec, generator];
        module
    }

    #[test]
    fn joint_fixpoint_inlines_calls_exposed_by_second_pass_templates() {
        let on = run(joint_fixpoint_module(), InlineFeatures::ALL);
        assert_eq!(calls_to(&on, "f", "gen"), 0);
        assert_eq!(calls_to(&on, "f", "ai"), 0, "always_inline contract");
        assert_eq!(calls_to(&on, "f", "leaf"), 0, "exposed tiny calls inline");
        let off = run(
            joint_fixpoint_module(),
            InlineFeatures {
                joint_fixpoint: false,
                ..InlineFeatures::ALL
            },
        );
        assert_eq!(calls_to(&off, "f", "ai"), 0, "always_inline contract");
        assert!(
            calls_to(&off, "f", "leaf") > 0,
            "negative control: without the joint fixpoint the exposed calls survive"
        );
    }

    /// zstd's HUF shape: one entry site whose always_inline body exposes many
    /// acyclic template sites (`HUF_compress1X_usingCTable_internal_body`:
    /// ten `body_loop` clones). A flat Phase 0 counter (2x entry + 16 steps)
    /// drained 17 of them, and the instruction-metered first and second
    /// passes (200 + 400) cannot absorb the remaining 23 x 30, so template
    /// calls stayed out of line. Acyclic expansion must drain completely.
    #[test]
    fn phase0_drains_acyclic_template_fanout_completely() {
        let mut template = function("template", 30, &[]);
        template.is_static = true;
        template.is_always_inline = true;
        let mut body = function("body", 0, &["template"; 40]);
        body.is_static = true;
        body.is_always_inline = true;
        let caller = function("f", 0, &["body"]);
        let mut module = IrModule::new();
        // `f` first: it clones the raw `body` (see joint_fixpoint_module).
        module.functions = vec![caller, template, body];
        let out = run(module, InlineFeatures::ALL);
        assert_eq!(calls_to(&out, "f", "body"), 0);
        assert_eq!(calls_to(&out, "f", "template"), 0, "always_inline contract");
    }

    /// A processed single-call-site static must be cloned as processed.
    /// `h` inlines all six calls to the ordinary (budgeted) static `helper`
    /// itself. Its only caller `g` first grows past the soft caller cap and
    /// the per-caller budget on `static inline` `other` sites (admitted
    /// regardless of both) while staying under the hard cap, which binds
    /// every non-always_inline callee. `h` still inlines (a single-call-site
    /// static within the loop-nest exemption cost skips the soft cap), but
    /// cloning the raw `h` would re-expose six `helper` calls that `g` can no
    /// longer afford, while the refreshed snapshot has none left.
    fn snapshot_refresh_module() -> IrModule {
        let mut helper = function("helper", 22, &[]);
        helper.is_static = true;
        let mut other = function("other", 22, &[]);
        other.is_static = true;
        other.is_inline = true;
        let mut h = function("h", 0, &["helper"; 6]);
        h.is_static = true;
        let mut calls = vec!["other"; 16];
        calls.push("h");
        let mut g = function("g", 0, &calls);
        g.is_noinline = true;
        let mut module = IrModule::new();
        module.functions = vec![helper, other, h, g];
        module
    }

    /// `helper` computes nothing observable: once inlined, all its work is
    /// dead. Processing `h` (called twice, so a grown body is never
    /// refreshed) therefore grows the unsimplified body by six dead copies of
    /// that work, while the cleaned body is smaller than the raw snapshot.
    /// Only the cleaned refresh spares `g` -- past its soft cap and budget --
    /// from re-exposed `helper` calls it can no longer afford.
    fn snapshot_cleanup_module() -> IrModule {
        let mut helper = IrFunction::new("helper".to_string(), IrType::Void, vec![], false);
        let instructions: Vec<Instruction> = (1..=22u32)
            .map(|i| Instruction::BinOp {
                dest: Value(i),
                op: IrBinOp::Add,
                lhs: if i == 1 {
                    Operand::Const(IrConst::I32(7))
                } else {
                    Operand::Value(Value(i - 1))
                },
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            })
            .collect();
        helper.next_value_id = 23;
        helper.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        helper.is_static = true;
        let mut other = function("other", 22, &[]);
        other.is_static = true;
        other.is_inline = true;
        let mut h = function("h", 3, &["helper"; 6]);
        h.is_static = true;
        let mut calls = vec!["other"; 16];
        calls.extend(["h", "h"]);
        let mut g = function("g", 3, &calls);
        g.is_noinline = true;
        let mut module = IrModule::new();
        module.functions = vec![helper, other, h, g];
        module
    }

    #[test]
    fn snapshot_cleanup_refreshes_a_body_that_only_grew_by_dead_work() {
        let on = run(snapshot_cleanup_module(), InlineFeatures::ALL);
        assert_eq!(calls_to(&on, "h", "helper"), 0, "h processes its helpers");
        assert_eq!(calls_to(&on, "g", "h"), 0, "both h sites inline");
        assert_eq!(calls_to(&on, "g", "helper"), 0, "cleaned body cloned");
        let off = run(
            snapshot_cleanup_module(),
            InlineFeatures {
                snapshot_cleanup: false,
                ..InlineFeatures::ALL
            },
        );
        assert_eq!(calls_to(&off, "g", "h"), 0, "both h sites inline");
        assert!(
            calls_to(&off, "g", "helper") > 0,
            "negative control: the grown unsimplified body is not refreshed"
        );
    }

    /// `waste` is 22 instructions of dead work, `static inline`, so the
    /// first selection pass admits all ten sites regardless of caller size.
    /// Pasted, they take `g` past the soft caller cap although all of it
    /// folds away. `helper` (plain static, 40 live instructions, two sites,
    /// not in a loop) is admitted only below the soft cap, so it inlines
    /// only when `g` is measured after an in-place cleanup.
    fn caller_cleanup_module() -> IrModule {
        let mut waste = snapshot_cleanup_module().functions.remove(0);
        waste.name = "waste".to_string();
        waste.is_inline = true;
        let mut helper = function("helper", 40, &[]);
        helper.is_static = true;
        let mut calls = vec!["waste"; 10];
        calls.extend(["helper", "helper"]);
        let mut g = function("g", 3, &calls);
        g.is_noinline = true;
        let mut module = IrModule::new();
        module.functions = vec![waste, helper, g];
        module
    }

    #[test]
    fn caller_cleanup_measures_a_capped_caller_after_folding_its_dead_work() {
        let on = run(caller_cleanup_module(), InlineFeatures::ALL);
        assert_eq!(calls_to(&on, "g", "waste"), 0, "first-pass sites inline");
        assert_eq!(
            calls_to(&on, "g", "helper"),
            0,
            "cleaned caller admits helper"
        );
        let g = on.functions.iter().find(|f| f.name == "g").unwrap();
        let stores = g
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| matches!(i, Instruction::Store { .. }))
            .count();
        assert_eq!(stores, 3, "g's own store and both helper clones survive");
        let off = run(
            caller_cleanup_module(),
            InlineFeatures {
                caller_cleanup: false,
                ..InlineFeatures::ALL
            },
        );
        assert_eq!(calls_to(&off, "g", "waste"), 0, "first-pass sites inline");
        assert_eq!(
            calls_to(&off, "g", "helper"),
            2,
            "negative control: the pasted size keeps g past the soft cap"
        );
    }

    #[test]
    fn snapshot_refresh_clones_the_processed_single_site_body() {
        let on = run(snapshot_refresh_module(), InlineFeatures::ALL);
        assert_eq!(calls_to(&on, "h", "helper"), 0, "h processes its helpers");
        assert_eq!(calls_to(&on, "g", "h"), 0, "single call site inlines");
        assert_eq!(calls_to(&on, "g", "helper"), 0, "processed body cloned");
        let off = run(
            snapshot_refresh_module(),
            InlineFeatures {
                snapshot_refresh: false,
                ..InlineFeatures::ALL
            },
        );
        assert_eq!(calls_to(&off, "g", "h"), 0, "single call site inlines");
        assert!(
            calls_to(&off, "g", "helper") > 0,
            "negative control: the raw snapshot re-exposes expanded calls"
        );
    }
}

#[cfg(test)]
mod extern_admission_tests {
    use super::*;
    use crate::common::pass_policy::Libcall;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, IrConst, Operand, Terminator};

    /// A leaf, loop-free, scalar-returning, global (non-static) definition.
    fn leaf_definition(name: &str) -> IrFunction {
        let mut func = IrFunction::new(name.into(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 2;
        func.next_label = 2;
        func
    }

    fn quiet_context() -> CalleeContext {
        CalleeContext {
            call_counts: Default::default(),
            value_referenced: Default::default(),
            asm_templates: Vec::new(),
            call_site_counts: Default::default(),
            sole_caller: Default::default(),
            debug_callee: false,
            semantic_interposition: false,
        }
    }

    /// A known external leaf is admitted irrespective of its spelling.  Libcall
    /// synthesis safety is a per-TU `BuiltinPolicy` fact applied before this
    /// phase and copied with the body; the inliner must not add a name-based
    /// performance cliff after that policy has already proved the body safe.
    #[test]
    fn global_leaf_admission_is_not_name_based() {
        for name in std::iter::once("helper").chain(Libcall::ALL.iter().map(|l| l.name())) {
            assert!(
                extern_inline_admission(&leaf_definition(name), &quiet_context()),
                "global leaf `{name}` should be admitted"
            );
        }
    }

    #[test]
    fn elides_readonly_local_memcpy_in_call_free_inlined_body() {
        use crate::common::types::AddressSpace;

        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(10),
                    size: 8,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(3),
                    ptr: Value(2),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            !blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { .. })),
            "readonly local memcpy must be elided"
        );
        assert!(
            blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::GetElementPtr {
                    base: Value(10),
                    ..
                }
            )),
            "GEP base must be re-rooted at memcpy source"
        );
    }

    /// One-block body: `alloca A(8); memcpy(A, S, 8); gep G = A+0; store 1, G`.
    /// The store writes *through* an alias of the local, so the copy cannot be
    /// elided: there would be no destination left to write.
    #[test]
    fn refuses_to_elide_when_a_field_is_written_through_an_alias() {
        use crate::common::types::AddressSpace;

        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(10),
                    size: 8,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(0)),
                    ty: IrType::I64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(1)),
                    ptr: Value(2),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { dest: Value(1), .. })),
            "a local written through an alias must keep its alloca"
        );
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { dest: Value(1), .. })),
            "a local written through an alias must keep its memcpy"
        );
    }

    /// The alias closure must not depend on block *order*. `BasicBlock` order
    /// is not dominance order: after inlining, the block that stores through a
    /// `GetElementPtr` can precede the block that defines it. Judging the store
    /// before discovering the alias (a single interleaved pass) elides the
    /// local and leaves the store writing into the *source* object.
    #[test]
    fn alias_discovery_is_order_independent_across_blocks() {
        use crate::common::types::AddressSpace;

        let alloca_and_copy = vec![
            Instruction::Alloca {
                dest: Value(1),
                ty: IrType::Ptr,
                size: 8,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(10),
                size: 8,
            },
        ];
        let mut blocks = vec![
            // entry: the candidate local
            BasicBlock {
                label: BlockId(0),
                instructions: alloca_and_copy,
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
            },
            // uses an alias defined only in block 2
            BasicBlock {
                label: BlockId(1),
                instructions: vec![Instruction::Store {
                    val: Operand::Const(IrConst::I64(1)),
                    ptr: Value(2),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                }],
                terminator: Terminator::Branch(BlockId(2)),
                source_spans: Vec::new(),
            },
            // defines the alias
            BasicBlock {
                label: BlockId(2),
                instructions: vec![Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(0)),
                    ty: IrType::I64,
                }],
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            },
        ];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { dest: Value(1), .. })),
            "an escape discovered in a later block still disqualifies the local"
        );
    }

    /// A `Copy` of the pointer is the same address, so it must join the alias
    /// closure instead of counting as an escape. `mov`-style copies of the
    /// alloca are routine in lowered IR; disqualifying them lost the elision.
    #[test]
    fn copy_of_the_local_is_an_alias_not_an_escape() {
        use crate::common::types::AddressSpace;

        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(10),
                    size: 8,
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                },
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(2),
                    offset: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(4),
                    ptr: Value(3),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            !blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { .. })),
            "a copied pointer is still an alias, not an escape"
        );
        assert!(
            !blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { .. })),
            "the elided local's alloca must be removed"
        );
        assert!(
            blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    src: Operand::Value(Value(10)),
                    ..
                }
            )),
            "the pointer copy must be re-rooted at the memcpy source"
        );
    }

    /// SROA-unlock candidates must use the *same* definition of "parameter
    /// home" that `aggregate_sroa` uses (`param_alloca_values`), not entry-block
    /// position: the inliner must never promise an unlock the splitter refuses.
    #[test]
    fn sroa_aliases_exclude_param_homes_recorded_by_the_block() {
        use crate::common::types::AddressSpace;

        let body = || {
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(3),
                    ptr: Value(2),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ]
        };
        let build = |param_home: bool| {
            let mut func = IrFunction::new("f".into(), IrType::I64, vec![], false);
            if param_home {
                func.param_alloca_values = vec![Value(1)];
            }
            func.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: body(),
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            });
            func
        };
        let callees: FxHashMap<String, CalleeData> = FxHashMap::default();
        assert!(
            sroa_unlockable_aliases_of(&build(true), &callees, usize::MAX).is_empty(),
            "a parameter home is never an SROA unlock candidate"
        );
        let locals = sroa_unlockable_aliases_of(&build(false), &callees, usize::MAX);
        assert!(
            locals.contains(&Value(1)) && locals.contains(&Value(2)),
            "a non-escaping local aggregate and its GEP alias are candidates: {locals:?}"
        );
    }

    /// Two locals chained by copies (the by-value struct parameter bridge the
    /// inliner builds) must collapse to one source. A one-step rewrite would
    /// leave the outer local's readers pointing at the inner local after the
    /// inner one was removed.
    #[test]
    fn chained_copies_collapse_to_the_ultimate_source() {
        use crate::common::types::AddressSpace;

        let alloca = |dest: u32| Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size: 8,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        };
        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(1), // outer local
                alloca(2), // inner local
                Instruction::Memcpy {
                    dest: Value(2),
                    src: Value(10),
                    size: 8,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(2),
                    size: 8,
                },
                Instruction::Load {
                    dest: Value(3),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            !blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { .. })),
            "both locals and both copies must be elided"
        );
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Load { ptr: Value(10), .. })),
            "the reader must land on the ultimate source in one step"
        );
    }

    /// A copy whose destination is refused survives, and it reads its source
    /// local when it runs. Re-rooting it at the ultimate source would read
    /// that source LATE (after any write between the two copies), so the
    /// local it reads is kept, with its own copy, instead.
    #[test]
    fn surviving_copy_keeps_the_local_it_reads() {
        use crate::common::types::AddressSpace;

        let alloca = |dest: u32| Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size: 8,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        };
        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(1), // read into, then written through: refused
                alloca(2), // read-only: elided
                Instruction::Memcpy {
                    dest: Value(2),
                    src: Value(10),
                    size: 8,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(2),
                    size: 8,
                },
                // Refuses local 1 (a write through it) but leaves local 2
                // read-only. The copy above is therefore not the elided copy
                // of local 1 and survives -- while reading the elided local 2.
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(1)),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { dest: Value(1), .. })),
            "the written-through local must survive"
        );
        assert!(
            blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { dest: Value(2), .. })),
            "the local a surviving copy reads must be kept"
        );
        for (dest, src) in [(2, 10), (1, 2)] {
            assert!(
                blocks[0].instructions.iter().any(|i| matches!(
                    i,
                    Instruction::Memcpy { dest: Value(d), src: Value(s), .. } if *d == dest && *s == src
                )),
                "copy {dest} <- {src} must be unchanged"
            );
        }
    }

    // ── snapshot semantics: readers move to the copy ────────────────────────

    fn elide_alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn elide_copy(dest: u32, src: u32, size: usize) -> Instruction {
        Instruction::Memcpy {
            dest: Value(dest),
            src: Value(src),
            size,
        }
    }

    fn elide_gep(dest: u32, base: u32, offset: Operand) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset,
            ty: IrType::I32,
        }
    }

    fn elide_load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        }
    }

    fn elide_store(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Const(IrConst::I32(99)),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        }
    }

    fn elide_block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn has_copy_into(blocks: &[BasicBlock], dest: u32) -> bool {
        blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { dest: Value(d), .. } if *d == dest))
        })
    }

    /// `E e = *t; t->a = 99; return e.a;` -- the source is written after the
    /// copy and the read sits in a later block. The read must move to the
    /// copy (and read the source there), never stay behind the store.
    #[test]
    fn reader_moves_to_the_copy_ahead_of_a_source_write() {
        let mut blocks = vec![
            elide_block(
                0,
                vec![elide_alloca(1, 8), elide_copy(1, 10, 8), elide_store(10)],
                Terminator::Branch(BlockId(1)),
            ),
            elide_block(
                1,
                vec![elide_load(3, 1, IrType::I32)],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(!has_copy_into(&blocks, 1), "the copy is elided");
        let entry = &blocks[0].instructions;
        let load = entry
            .iter()
            .position(|i| {
                matches!(
                    i,
                    Instruction::Load {
                        dest: Value(3),
                        ptr: Value(10),
                        ..
                    }
                )
            })
            .expect("the read moved to the entry block, re-rooted at the source");
        let store = entry
            .iter()
            .position(|i| matches!(i, Instruction::Store { .. }))
            .expect("the store stays");
        assert!(
            load < store,
            "the read must precede the write to the source"
        );
        assert!(blocks[1].instructions.is_empty(), "nothing left behind");
    }

    /// A second copy into the local refills it with newer bytes, even from
    /// the same source value; the local is no longer a single snapshot.
    #[test]
    fn second_copy_from_the_same_source_refuses() {
        let mut blocks = vec![
            elide_block(
                0,
                vec![elide_alloca(1, 8), elide_copy(1, 10, 8), elide_store(10)],
                Terminator::Branch(BlockId(1)),
            ),
            elide_block(
                1,
                vec![elide_copy(1, 10, 8), elide_load(3, 1, IrType::I32)],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        elide_readonly_local_memcpys(&mut blocks);
        assert!(has_copy_into(&blocks, 1), "both copies must stay");
    }

    /// Readers that cannot move to the copy keep the local: a runtime offset
    /// (its operand may not exist at the copy), a load reaching past the
    /// copied bytes (it would read memory the copy never dereferenced), and
    /// the local's address used as an integer offset (an escape).
    #[test]
    fn unmovable_readers_refuse() {
        let shapes: Vec<Vec<Instruction>> = vec![
            vec![
                elide_gep(2, 1, Operand::Value(Value(20))),
                elide_load(3, 2, IrType::I32),
            ],
            vec![
                elide_gep(2, 1, Operand::Const(IrConst::I64(6))),
                elide_load(3, 2, IrType::I32),
            ],
            vec![
                elide_gep(2, 30, Operand::Value(Value(1))),
                elide_load(3, 2, IrType::I32),
            ],
        ];
        for (n, tail) in shapes.into_iter().enumerate() {
            let mut insts = vec![elide_alloca(1, 8), elide_copy(1, 10, 8)];
            insts.extend(tail);
            let mut blocks = vec![elide_block(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(3)))),
            )];
            elide_readonly_local_memcpys(&mut blocks);
            assert!(has_copy_into(&blocks, 1), "shape {n} must keep the copy");
        }
    }

    /// `L2 <- L1 <- p` with `p` written between the two copies: `L2` holds
    /// what `L1` took from `p` FIRST, so L2's reader is snapshotted at L1's
    /// copy, ahead of the store.
    #[test]
    fn chain_is_snapshotted_at_its_first_copy() {
        let mut blocks = vec![elide_block(
            0,
            vec![
                elide_alloca(1, 8),
                elide_alloca(2, 8),
                elide_copy(1, 10, 8),
                elide_store(10),
                elide_copy(2, 1, 8),
                elide_load(3, 2, IrType::I32),
            ],
            Terminator::Return(Some(Operand::Value(Value(3)))),
        )];
        elide_readonly_local_memcpys(&mut blocks);
        let entry = &blocks[0].instructions;
        assert!(!has_copy_into(&blocks, 1) && !has_copy_into(&blocks, 2));
        let load = entry
            .iter()
            .position(|i| matches!(i, Instruction::Load { ptr: Value(10), .. }))
            .expect("the read lands on the ultimate source");
        let store = entry
            .iter()
            .position(|i| matches!(i, Instruction::Store { .. }))
            .unwrap();
        assert!(load < store, "snapshot at the first copy, before the store");
    }

    /// Debug locations must survive the rewrite: dropping the span vector
    /// silently loses `-g` line info for the whole block.
    #[test]
    fn elision_keeps_source_spans_parallel_to_instructions() {
        use crate::common::types::AddressSpace;

        let span = |start: u32| crate::common::source::Span::new(start, start + 1, 0);
        let mut blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(10),
                    size: 8,
                },
                Instruction::Load {
                    dest: Value(2),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: vec![span(1), span(2), span(3)],
        }];
        elide_readonly_local_memcpys(&mut blocks);
        assert_eq!(blocks[0].instructions.len(), 1);
        assert_eq!(blocks[0].source_spans.len(), 1);
        assert_eq!(blocks[0].source_spans[0].start, 3);
    }

    #[test]
    fn devirtualizes_entry_alloca_and_recursive_invariant_fn_ptr_param() {
        use crate::common::types::AddressSpace;
        use crate::ir::reexports::{CallInfo, IrModule, IrParam};

        let mut target = IrFunction::new("cmp_fn".to_string(), IrType::I32, vec![], false);
        target.is_static = true;
        target.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        // Callee `worker(fn_ptr)` calls `fn_ptr` indirectly and recurses with
        // the same `ParamRef(0)` argument, loading it through an unescaped
        // entry alloca.
        let mut worker = IrFunction::new(
            "worker".to_string(),
            IrType::I32,
            vec![IrParam {
                ty: IrType::Ptr,
                struct_size: None,
                struct_align: None,
                struct_eightbyte_classes: Vec::new(),
                riscv_float_class: None,
                is_f128_sse: false,
                noalias: false,
                param_align: None,
            }],
            false,
        );
        worker.is_static = true;
        worker.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Value(Value(1)),
                    ptr: Value(0),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(2),
                    ptr: Value(0),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: CallInfo {
                        dest: Some(Value(3)),
                        args: vec![Operand::Value(Value(2))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::I32,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![None],
                        struct_arg_aligns: vec![None],
                        struct_arg_classes: vec![Vec::new()],
                        struct_arg_riscv_float_classes: vec![None],
                        is_sret: false,
                        calling_convention: crate::common::types::CallingConvention::Default,
                        is_fastcall: false,
                        ret_eightbyte_classes: Vec::new(),
                        ..CallInfo::default()
                    },
                },
                Instruction::CallIndirect {
                    func_ptr: Operand::Value(Value(2)),
                    info: CallInfo {
                        dest: Some(Value(4)),
                        args: Vec::new(),
                        arg_types: Vec::new(),
                        return_type: IrType::I32,
                        is_variadic: false,
                        num_fixed_args: 0,
                        struct_arg_sizes: Vec::new(),
                        struct_arg_aligns: Vec::new(),
                        struct_arg_classes: Vec::new(),
                        struct_arg_riscv_float_classes: Vec::new(),
                        is_sret: false,
                        calling_convention: crate::common::types::CallingConvention::Default,
                        is_fastcall: false,
                        ret_eightbyte_classes: Vec::new(),
                        ..CallInfo::default()
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        });

        let mut caller = IrFunction::new("caller".to_string(), IrType::I32, vec![], false);
        caller.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "cmp_fn".to_string(),
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: CallInfo {
                        dest: Some(Value(1)),
                        args: vec![Operand::Value(Value(0))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::I32,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![None],
                        struct_arg_aligns: vec![None],
                        struct_arg_classes: vec![Vec::new()],
                        struct_arg_riscv_float_classes: vec![None],
                        is_sret: false,
                        calling_convention: crate::common::types::CallingConvention::Default,
                        is_fastcall: false,
                        ret_eightbyte_classes: Vec::new(),
                        ..CallInfo::default()
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });

        let mut module = IrModule::new();
        module.functions = vec![target, worker, caller];
        assert_eq!(devirtualize_constant_indirect_calls(&mut module).len(), 1);
        assert!(matches!(
            &module.functions[1].blocks[0].instructions[5],
            Instruction::Call { func, .. } if func == "cmp_fn"
        ));
    }

    // ---- invariant-callback devirtualization: the refusal space ----
    //
    // `propagate_invariant_fn_ptr_params` is sound only because every one
    // of its guards refuses.  Each test below builds the proven-positive
    // module and then breaks exactly one guard; deleting or weakening that
    // guard must turn exactly its test red.  Positive control:
    // `devirtualizes_entry_alloca_and_recursive_invariant_fn_ptr_param`.

    fn devirt_ptr_param() -> crate::ir::reexports::IrParam {
        crate::ir::reexports::IrParam {
            ty: IrType::Ptr,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: Vec::new(),
            riscv_float_class: None,
            is_f128_sse: false,
            noalias: false,
            param_align: None,
        }
    }

    fn devirt_call_info(
        dest: Value,
        args: Vec<Operand>,
        arg_types: Vec<IrType>,
    ) -> crate::ir::reexports::CallInfo {
        use crate::ir::reexports::CallInfo;
        let n = arg_types.len();
        CallInfo {
            dest: Some(dest),
            args,
            arg_types,
            return_type: IrType::I32,
            is_variadic: false,
            num_fixed_args: n,
            struct_arg_sizes: vec![None; n],
            struct_arg_aligns: vec![None; n],
            struct_arg_classes: vec![Vec::new(); n],
            struct_arg_riscv_float_classes: vec![None; n],
            is_sret: false,
            calling_convention: crate::common::types::CallingConvention::Default,
            is_fastcall: false,
            ret_eightbyte_classes: Vec::new(),
            ..CallInfo::default()
        }
    }

    /// The positive fixture: `caller` passes `cmp_fn` to static `worker`,
    /// which loads it through an unescaped entry alloca, recurses with it,
    /// and calls it indirectly.  Function order: [cmp_fn, worker, caller].
    fn invariant_callback_module() -> crate::ir::reexports::IrModule {
        use crate::common::types::AddressSpace;
        use crate::ir::reexports::IrModule;

        let mut target = IrFunction::new("cmp_fn".to_string(), IrType::I32, vec![], false);
        target.is_static = true;
        target.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        let mut worker = IrFunction::new(
            "worker".to_string(),
            IrType::I32,
            vec![devirt_ptr_param()],
            false,
        );
        worker.is_static = true;
        worker.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Value(Value(1)),
                    ptr: Value(0),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(2),
                    ptr: Value(0),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: devirt_call_info(
                        Value(3),
                        vec![Operand::Value(Value(2))],
                        vec![IrType::Ptr],
                    ),
                },
                Instruction::CallIndirect {
                    func_ptr: Operand::Value(Value(2)),
                    info: devirt_call_info(Value(4), vec![], vec![]),
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        });

        let mut caller = IrFunction::new("caller".to_string(), IrType::I32, vec![], false);
        caller.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "cmp_fn".to_string(),
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: devirt_call_info(
                        Value(1),
                        vec![Operand::Value(Value(0))],
                        vec![IrType::Ptr],
                    ),
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });

        let mut module = IrModule::new();
        module.functions = vec![target, worker, caller];
        module
    }

    fn assert_not_devirtualized(module: &mut crate::ir::reexports::IrModule) {
        assert!(devirtualize_constant_indirect_calls(module).is_empty());
        assert!(
            module.functions[1].blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::CallIndirect { .. })),
            "indirect call must survive"
        );
    }

    #[test]
    fn devirt_refuses_disagreeing_second_caller() {
        let mut module = invariant_callback_module();
        let mut other = IrFunction::new("other_fn".to_string(), IrType::I32, vec![], false);
        other.is_static = true;
        other.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            source_spans: Vec::new(),
        });
        let mut caller2 = IrFunction::new("caller2".to_string(), IrType::I32, vec![], false);
        caller2.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "other_fn".to_string(),
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: devirt_call_info(
                        Value(1),
                        vec![Operand::Value(Value(0))],
                        vec![IrType::Ptr],
                    ),
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });
        module.functions.push(other);
        module.functions.push(caller2);
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_weak_callee() {
        let mut module = invariant_callback_module();
        module.functions[1].is_weak = true;
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_variadic_callee() {
        let mut module = invariant_callback_module();
        module.functions[1].is_variadic = true;
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_external_linkage_callee() {
        let mut module = invariant_callback_module();
        module.functions[1].is_static = false;
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_asm_labelled_callee() {
        let mut module = invariant_callback_module();
        module
            .asm_labels
            .insert("worker".to_string(), "worker_asm".to_string());
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_arity_mismatched_call_site() {
        let mut module = invariant_callback_module();
        match &mut module.functions[2].blocks[0].instructions[1] {
            Instruction::Call { info, .. } => {
                info.args.push(Operand::Const(IrConst::I64(0)));
                info.arg_types.push(IrType::I64);
            }
            _ => panic!("fixture shape changed"),
        }
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_twice_stored_entry_alloca() {
        let mut module = invariant_callback_module();
        let store = module.functions[1].blocks[0].instructions[2].clone();
        module.functions[1].blocks[0].instructions.insert(3, store);
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_load_before_store_entry_alloca() {
        let mut module = invariant_callback_module();
        module.functions[1].blocks[0].instructions.swap(2, 3);
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_entry_block_back_edge() {
        let mut module = invariant_callback_module();
        module.functions[1].blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(0)),
            source_spans: Vec::new(),
        });
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_refuses_unresolved_parameter_origin() {
        let mut module = invariant_callback_module();
        module.functions[2].params.push(devirt_ptr_param());
        module.functions[2].blocks[0].instructions[0] = Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        };
        assert_not_devirtualized(&mut module);
    }

    #[test]
    fn devirt_composes_through_passthrough_caller() {
        let mut module = invariant_callback_module();
        // caller -> mid(cmp_fn); mid forwards its parameter to worker.
        match &mut module.functions[2].blocks[0].instructions[1] {
            Instruction::Call { func, .. } => *func = "mid".to_string(),
            _ => panic!("fixture shape changed"),
        }
        let mut mid = IrFunction::new(
            "mid".to_string(),
            IrType::I32,
            vec![devirt_ptr_param()],
            false,
        );
        mid.is_static = true;
        mid.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Call {
                    func: "worker".to_string(),
                    info: devirt_call_info(
                        Value(1),
                        vec![Operand::Value(Value(0))],
                        vec![IrType::Ptr],
                    ),
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });
        module.functions.push(mid);
        let rewritten = devirtualize_constant_indirect_calls(&mut module);
        assert!(rewritten.contains(&("worker".to_string(), "cmp_fn".to_string())));
    }
}

#[cfg(test)]
mod target_isa_tests {
    use super::{IsaCallee, x86_features_allow_inline};
    use crate::backend::x86::isa::X86Isa;
    use crate::common::x86_features::X86Feature;
    use crate::ir::module::normalize_target_features;

    fn set(raw: &str) -> Vec<String> {
        normalize_target_features(raw)
    }

    fn cpu(name: &str) -> u64 {
        X86Isa::arch_row(name).expect("GCC CPU").feature_bits()
    }

    /// May `callee` (with its own `target(...)`) inline into the plain
    /// function `caller` in an x86-64 TU with features `tu`?
    fn allows(caller: &str, callee: &str, tu: u64) -> bool {
        let callee = set(callee);
        let side = IsaCallee {
            features: &callee,
            needs: 0,
            uses_fp_or_simd: false,
        };
        x86_features_allow_inline(&set(caller), false, side, tu, 0, true)
    }

    /// May a callee without `target(...)` that needs `needs` (and, with
    /// `fp`, the FP/SIMD registers) inline into `caller`?
    fn plain_allows(caller: &str, always_inline: bool, needs: u64, fp: bool, tu: u64) -> bool {
        let side = IsaCallee {
            features: &[],
            needs,
            uses_fp_or_simd: fp,
        };
        x86_features_allow_inline(&set(caller), always_inline, side, tu, 0, true)
    }

    fn bits(features: &[X86Feature]) -> u64 {
        features.iter().fold(0, |b, &f| b | 1 << f as u8)
    }

    #[test]
    fn normalization_keeps_the_attribute_order() {
        assert_eq!(
            set(" BMI2 ,lzcnt,,bmi, bmi2 "),
            ["bmi2", "lzcnt", "bmi", "bmi2"]
        );
        assert!(set(" , ").is_empty());
    }

    /// GCC 16.2: `arch=` discards the ISA options before it (grants,
    /// denials, `general-regs-only`) and keeps those after it and every
    /// `key=value` option.
    #[test]
    fn arch_discards_the_isa_options_before_it() {
        assert_eq!(set("avx2,arch=x86-64"), ["arch=x86-64"]);
        assert_eq!(
            set("no-avx2,general-regs-only,arch=haswell,no-fma"),
            ["arch=haswell", "no-fma"]
        );
        assert_eq!(
            set("tune=znver4,avx2,arch=x86-64,popcnt"),
            ["tune=znver4", "arch=x86-64", "popcnt"]
        );
        assert_eq!(set("bmi2,lzcnt"), ["bmi2", "lzcnt"]);
    }

    /// A body without `target(...)` carries only what its IR depends on.
    #[test]
    fn plain_callee_needs_only_what_its_ir_depends_on() {
        use X86Feature as F;
        let (v1, haswell) = (cpu("x86-64"), cpu("haswell"));
        // `_mm_popcnt_u32` (`__builtin_popcount`) depends on nothing: it
        // reaches a haswell unit's dispatch baseline and becomes the
        // portable sequence there.
        assert!(plain_allows("arch=x86-64", false, 0, false, haswell));
        assert!(plain_allows("general-regs-only", false, 0, false, v1));
        // A helper that lowered an AVX2 intrinsic stays a call there.
        let avx2 = bits(&[F::Avx2]);
        assert!(!plain_allows("arch=x86-64", false, avx2, false, haswell));
        assert!(plain_allows("arch=haswell", false, avx2, false, haswell));
        assert!(plain_allows("", false, avx2, false, haswell));
        // FP and SIMD values need the register files the caller removes.
        assert!(!plain_allows("general-regs-only", false, 0, true, v1));
        assert!(!plain_allows("no-sse2", false, 0, true, v1));
        assert!(!plain_allows("no-80387", false, 0, true, v1));
        assert!(plain_allows("bmi2", false, 0, true, v1));
        assert!(plain_allows("no-80387,80387", false, 0, true, v1));
    }

    /// lccc's 512-bit intrinsics are `always_inline` header bodies whose
    /// requirement a `-march=x86-64` unit defers to the inlining caller.
    #[test]
    fn deferred_requirements_settle_in_a_granting_caller() {
        use X86Feature as F;
        let v1 = cpu("x86-64");
        let bw = bits(&[F::Avx512Bw, F::Avx512F]);
        assert!(plain_allows("avx512bw", false, bw, true, v1));
        assert!(plain_allows("avx512bmm", false, bw, true, v1));
        assert!(!plain_allows("avx512f", false, bw, true, v1));
        assert!(!plain_allows("", false, bw, true, v1));
        // Another header body takes it over and is checked where it lands.
        assert!(plain_allows("", true, bw, true, v1));
        // A `target(...)` always_inline caller is no deferring header body.
        assert!(!plain_allows("avx2", true, bw, true, v1));
    }

    /// Absorbing a body settles exactly the deferred features the caller
    /// grants, and keeps the first intrinsic that needs what remains.
    #[test]
    fn absorbing_settles_granted_deferred_features() {
        use crate::ir::module::X86IsaNeeds;
        use X86Feature as F;
        let inlined = X86IsaNeeds {
            relied: bits(&[F::Sse2]),
            deferred: bits(&[F::Avx512Bw, F::Avx512F]),
            first_use: Some("_mm512_add_epi8".to_string()),
        };
        let mut granting = X86IsaNeeds::default();
        granting.absorb(&inlined, bits(&[F::Sse2, F::Avx512Bw, F::Avx512F]));
        assert_eq!((granting.relied, granting.deferred), (bits(&[F::Sse2]), 0));
        assert_eq!(granting.first_use, None);
        let mut header = X86IsaNeeds::default();
        header.absorb(&inlined, cpu("x86-64"));
        assert_eq!(header.deferred, inlined.deferred);
        assert_eq!(header.first_use.as_deref(), Some("_mm512_add_epi8"));
    }

    #[test]
    fn target_callee_needs_a_covering_caller() {
        let v1 = cpu("x86-64");
        // zstd: plain dispatcher -> static BMI2 clone must stay a call.
        assert!(!allows("", "lzcnt,bmi,bmi2", v1));
        // Partial cover is not enough: one missing feature refuses.
        assert!(!allows("bmi,bmi2", "lzcnt,bmi,bmi2", v1));
        // Equal and superset callers accept, independent of spelling order.
        assert!(allows("bmi2,bmi,lzcnt", "lzcnt,bmi,bmi2", v1));
        assert!(allows("avx2,bmi,bmi2", "bmi2", v1));
        // Features compare, not spellings: `avx2` covers `sse4.2`, and
        // `arch=haswell` covers `avx2,bmi2`, but not the reverse.
        assert!(allows("avx2", "sse4.2,popcnt", v1));
        assert!(allows("arch=haswell", "avx2,bmi2", v1));
        assert!(!allows("bmi2", "arch=haswell", v1));
        assert!(!allows("avx2", "avx512bmm", v1));
        // A callee's denial needs no cover.
        assert!(allows("", "no-avx2", cpu("haswell")));
    }
}

#[cfg(test)]
mod phase0_tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, IrConst, Operand, Terminator};

    /// A defined function whose single block calls `callees` in order and
    /// then pads to `extra` copies, so its IR size is controllable.
    fn body(name: &str, callees: &[&str], extra: u32) -> IrFunction {
        let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
        let mut instructions: Vec<Instruction> = callees
            .iter()
            .map(|c| Instruction::Call {
                func: (*c).to_string(),
                info: CallInfo::default(),
            })
            .collect();
        instructions.extend((0..extra).map(|i| Instruction::Copy {
            dest: Value(i + 1),
            src: Operand::Const(IrConst::I32(i as i32)),
        }));
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions,
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = extra + 2;
        f.next_label = 2;
        f
    }

    fn quiet_context() -> CalleeContext {
        CalleeContext {
            call_counts: Default::default(),
            value_referenced: Default::default(),
            asm_templates: Vec::new(),
            call_site_counts: Default::default(),
            sole_caller: Default::default(),
            debug_callee: false,
            semantic_interposition: false,
        }
    }

    #[test]
    fn call_cycles_cover_self_mutual_and_long_recursion_only() {
        let mut module = IrModule::new();
        for (name, callees) in [
            ("a", &["b"][..]),
            ("b", &["c", "ext"][..]),
            ("c", &[][..]),
            ("self_rec", &["self_rec"][..]),
            ("even", &["odd"][..]),
            ("odd", &["even"][..]),
            ("x", &["y"][..]),
            ("y", &["z"][..]),
            ("z", &["x", "c"][..]),
        ] {
            module.functions.push(body(name, callees, 0));
        }
        let mut ext = IrFunction::new("ext".into(), IrType::I32, vec![], false);
        ext.is_declaration = true;
        module.functions.push(ext);
        let mut cyclic: Vec<String> = functions_on_call_cycles(&module).into_iter().collect();
        cyclic.sort();
        assert_eq!(cyclic, ["even", "odd", "self_rec", "x", "y", "z"]);
    }

    /// GCC inlines always_inline regardless of size; the old 500-instruction
    /// cap left zstd's FORCE_INLINE templates (803 IR instructions) out of
    /// line. Only the resource guard may refuse.
    #[test]
    fn always_inline_admission_is_bounded_only_by_the_resource_guard() {
        let module = IrModule::new();
        let mut big = body("template", &[], 900);
        big.is_static = true;
        big.is_inline = true;
        big.is_always_inline = true;
        let data = callee_data_for(&big, &module, &quiet_context());
        assert!(
            data.is_some_and(|d| d.is_always_inline),
            "900-instruction always_inline refused"
        );

        let guard = u32::try_from(ALWAYS_INLINE_GUARD_INSTRUCTIONS).unwrap();
        let mut huge = body("generated", &[], guard + 1);
        huge.is_static = true;
        huge.is_inline = true;
        huge.is_always_inline = true;
        assert!(callee_data_for(&huge, &module, &quiet_context()).is_none());
    }

    fn static_leaf(name: &str, callees: &[&str], extra: u32) -> IrFunction {
        let mut f = body(name, callees, extra);
        f.is_static = true;
        f
    }

    fn sites(module: &IrModule, caller: &str, callee: &str) -> usize {
        module
            .functions
            .iter()
            .filter(|f| f.name == caller)
            .flat_map(|f| f.blocks.iter().flat_map(|b| b.instructions.iter()))
            .filter(|i| matches!(i, Instruction::Call { func, .. } if func == callee))
            .count()
    }

    /// Frame-neutral = one block, <= MAX_SMALL instructions, no stack
    /// object, no call. Each excluded property is pinned separately.
    #[test]
    fn frame_neutral_leaf_excludes_frames_calls_and_control_flow() {
        let module = IrModule::new();
        let ctx = quiet_context();
        let neutral = |f: &IrFunction| {
            let d = callee_data_for(f, &module, &ctx).expect("eligible callee");
            let count = d.blocks.iter().map(|b| b.instructions.len()).sum();
            is_frame_neutral_leaf(&d, count)
        };
        assert!(neutral(&static_leaf("w", &[], 12)));
        let small = u32::try_from(MAX_SMALL_INLINE_INSTRUCTIONS).unwrap();
        assert!(!neutral(&static_leaf("big", &[], small + 1)));
        assert!(!neutral(&static_leaf("calls", &["ext"], 12)));
        let mut frame = static_leaf("frame", &[], 12);
        frame.blocks[0].instructions.push(Instruction::Alloca {
            dest: Value(40),
            ty: IrType::I64,
            size: 64,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        });
        assert!(!neutral(&frame));
        let mut branchy = static_leaf("branchy", &[], 12);
        let mut tail = branchy.blocks[0].clone();
        tail.label = BlockId(2);
        tail.instructions.clear();
        branchy.blocks[0].terminator = Terminator::Branch(BlockId(2));
        branchy.blocks.push(tail);
        assert!(!neutral(&branchy));
    }

    /// zstd's `ZSTD_litLengthPrice` shape: a self-recursive caller of a
    /// 12-instruction leaf (above the 5-instruction tiny limit). The leaf is
    /// inlined, and the self call is expanded by `clone_self_recursion_sites`:
    /// the single site is *replaced* by a copy of the body, so the caller
    /// still has exactly one self call — nested one level deeper — while its
    /// size roughly doubles.
    #[test]
    fn recursive_caller_inlines_frame_neutral_leaf() {
        let mut module = IrModule::new();
        module.functions.push(body("rec", &["rec", "leaf"], 4));
        module.functions.push(static_leaf("leaf", &[], 12));
        run(&mut module);
        assert_eq!(sites(&module, "rec", "leaf"), 0, "leaf left outlined");
        assert_eq!(
            sites(&module, "rec", "rec"),
            1,
            "one self call stays one self call after a one-site expansion"
        );
        let size: usize = module
            .functions
            .iter()
            .find(|f| f.name == "rec")
            .map(|f| f.blocks.iter().map(|b| b.instructions.len()).sum())
            .unwrap_or(0);
        assert!(
            size > 5,
            "the self call was not expanded: rec is still {size} instructions"
        );
    }

    /// The self clone: a two-site (tree-shaped) body is expanded exactly one
    /// level — `N` sites come back as `N^2`, and `4` exceeds
    /// `MAX_SELF_CLONE_SITES`, so a second invocation cannot expand it again.
    /// A single-site body keeps expanding while it still fits the size caps,
    /// which is why the transform is bounded rather than one-shot, and why the
    /// growth must terminate.
    #[test]
    fn self_recursion_clone_is_bounded_and_skips_tail_calls() {
        let mut module = IrModule::new();
        module
            .functions
            .push(static_leaf("walk", &["walk", "walk"], 6));
        run(&mut module);
        assert_eq!(
            sites(&module, "walk", "walk"),
            4,
            "two self call sites must be expanded into four"
        );
        let size_after_first = module.functions[0]
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum::<usize>();
        run(&mut module);
        assert_eq!(
            sites(&module, "walk", "walk"),
            4,
            "a second invocation must not expand the tree shape again"
        );
        assert_eq!(
            module.functions[0]
                .blocks
                .iter()
                .map(|b| b.instructions.len())
                .sum::<usize>(),
            size_after_first,
            "the cloned tree shape is not a fixed point in either direction"
        );

        // A single-site body converges under the size caps instead.
        let mut module = IrModule::new();
        module.functions.push(static_leaf("chain", &["chain"], 6));
        for _ in 0..8 {
            run(&mut module);
        }
        let final_size: usize = module.functions[0]
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum();
        assert!(
            final_size <= MAX_SELF_CLONE_FINAL_INSTRUCTIONS,
            "repeated expansion must stop under MAX_SELF_CLONE_FINAL_INSTRUCTIONS (got {final_size})"
        );
        assert_eq!(
            sites(&module, "chain", "chain"),
            1,
            "a single self call stays a single (nested) self call"
        );

        // Tail position: `Return(Some(call result))` is TCE's shape.
        let mut tail = IrFunction::new("sum".into(), IrType::I32, vec![], false);
        tail.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Call {
                func: "sum".to_string(),
                info: CallInfo {
                    dest: Some(Value(7)),
                    ..CallInfo::default()
                },
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(7)))),
            source_spans: Vec::new(),
        });
        tail.next_value_id = 8;
        tail.next_label = 2;
        let mut module = IrModule::new();
        module.functions.push(tail);
        run(&mut module);
        assert_eq!(
            sites(&module, "sum", "sum"),
            1,
            "a tail self call belongs to tail_call_elim, not to the cloner"
        );
    }
}

/// The SROA-unlock cost model (`sroa_unlockable_aliases_of`): plain statics
/// qualify, the clone must be paid for by the accesses SROA removes and fit
/// the remaining budget, and a whole-object library memset mirrors
/// `aggregate_sroa` form 4's rule 5b.
#[cfg(test)]
mod sroa_unlock_tests {
    use super::*;

    /// A straight-line internal helper of `instructions` instructions.
    fn helper(instructions: usize, declared_inline: bool) -> CalleeData {
        CalleeData {
            target_features: Vec::new(),
            builtin_policy: Default::default(),
            ssp: Default::default(),
            blocks: vec![BasicBlock {
                label: BlockId(0),
                instructions: (0..instructions)
                    .map(|i| Instruction::Copy {
                        dest: Value(i as u32),
                        src: Operand::Const(IrConst::I64(0)),
                    })
                    .collect(),
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            }],
            param_struct_sizes: Vec::new(),
            param_alloca_values: Vec::new(),
            param_home_shapes: Vec::new(),
            return_type: IrType::Void,
            num_params: 1,
            next_value_id: instructions as u32,
            max_block_id: 0,
            single_call_site: false,
            agg_frame_bytes: 0,
            is_always_inline: false,
            x86_isa_needs: Default::default(),
            uses_fp_or_simd: false,
            exceeds_normal_limits: false,
            is_static_inline: declared_inline,
            is_gnu_inline_def: false,
            is_plain_static: !declared_inline,
            direct_call_count: 2,
            loop_bound_params: 0,
            const_trip_only: false,
            const_branch_params: 0,
            const_branch_only: false,
            has_inlineable_loop_descendant: false,
            has_inlined_calls: false,
            is_single_call_site_static: false,
            has_loops: false,
            is_recursive: false,
            has_vector_intrinsics: false,
            size_inline_cost: instructions,
            va_arg_pack: None,
        }
    }

    fn call(func: &str, result: Option<u32>, args: Vec<Operand>, lib: bool) -> Instruction {
        Instruction::Call {
            func: func.into(),
            info: CallInfo {
                dest: result.map(Value),
                arg_types: vec![IrType::Ptr; args.len()],
                args,
                return_type: IrType::Ptr,
                assume_library: lib,
                ..CallInfo::default()
            },
        }
    }

    /// `alloca v1 (16 bytes)`, `prefix`, then `accesses` loads of `v1`, then
    /// `suffix`; the function returns the last load.
    fn caller(prefix: Vec<Instruction>, accesses: u32, suffix: Vec<Instruction>) -> IrFunction {
        let mut insts = vec![Instruction::Alloca {
            dest: Value(1),
            ty: IrType::Ptr,
            size: 16,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }];
        insts.extend(prefix);
        for k in 0..accesses {
            insts.push(Instruction::Load {
                dest: Value(100 + k),
                ptr: Value(1),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
        }
        insts.extend(suffix);
        let mut func = IrFunction::new("f".into(), IrType::I64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(Operand::Value(Value(100)))),
            source_spans: Vec::new(),
        });
        func
    }

    fn map(entries: Vec<(&str, CalleeData)>) -> FxHashMap<String, CalleeData> {
        entries
            .into_iter()
            .map(|(n, d)| (n.to_string(), d))
            .collect()
    }

    fn helper_call() -> Instruction {
        call("h", None, vec![Operand::Value(Value(1))], false)
    }

    /// zstd's `HUF_initCStream(&bitC, ...)` is a plain `static`: the escape
    /// goes away by inlining whether or not the helper was declared `inline`.
    #[test]
    fn plain_static_helper_unlocks_when_paid_for() {
        for declared_inline in [false, true] {
            let callees = map(vec![("h", helper(10, declared_inline))]);
            let func = caller(vec![helper_call()], 12, vec![]);
            assert!(
                sroa_unlockable_aliases_of(&func, &callees, usize::MAX).contains(&Value(1)),
                "inline={declared_inline}"
            );
            assert!(is_sroa_unlockable_candidate(&callees["h"]));
        }
    }

    /// Cloning 10 instructions to remove 3 memory accesses is growth.
    #[test]
    fn unpaid_unlock_is_refused() {
        let callees = map(vec![("h", helper(10, false))]);
        let func = caller(vec![helper_call()], 3, vec![]);
        assert!(!sroa_unlockable_aliases_of(&func, &callees, usize::MAX).contains(&Value(1)));
    }

    /// The whole escape set is priced at once (a partial unlock leaves the
    /// object in memory) and must fit the remaining budget.
    #[test]
    fn unlock_cost_is_summed_and_bounded_by_the_allowance() {
        let callees = map(vec![("h", helper(10, false))]);
        let func = caller(vec![helper_call()], 30, vec![helper_call()]);
        assert!(!sroa_unlockable_aliases_of(&func, &callees, 19).contains(&Value(1)));
        assert!(sroa_unlockable_aliases_of(&func, &callees, 20).contains(&Value(1)));
        let cheap = caller(vec![helper_call()], 15, vec![helper_call()]);
        assert!(
            !sroa_unlockable_aliases_of(&cheap, &callees, usize::MAX).contains(&Value(1)),
            "two clones (20) are not paid for by 15 accesses"
        );
    }

    /// A helper larger than a whole per-caller budget is never a candidate,
    /// and a looping or `always_inline` helper is not one either.
    #[test]
    fn candidate_shape_limits() {
        assert!(!is_sroa_unlockable_candidate(&helper(
            MAX_SROA_UNLOCK_HELPER_INSTRUCTIONS + 1,
            false
        )));
        let mut looping = helper(10, false);
        looping.has_loops = true;
        assert!(!is_sroa_unlockable_candidate(&looping));
        let mut forced = helper(10, false);
        forced.is_always_inline = true;
        assert!(!is_sroa_unlockable_candidate(&forced));
    }

    /// `memset(&local, 0, sizeof local)` (in `HUF_initCStream`) is expanded
    /// by form 4, so it neither escapes the object nor costs a clone; a
    /// partial, non-library or result-using memset disqualifies it, exactly
    /// where form 4 refuses.
    #[test]
    fn memset_mirrors_the_splitter() {
        let callees = map(vec![("h", helper(10, false))]);
        let ms = |len: i64, lib: bool, result: Option<u32>| {
            call(
                "memset",
                result,
                vec![
                    Operand::Value(Value(1)),
                    Operand::Const(IrConst::I32(0)),
                    Operand::Const(IrConst::I64(len)),
                ],
                lib,
            )
        };
        let admitted = |func: &IrFunction| {
            sroa_unlockable_aliases_of(func, &callees, usize::MAX).contains(&Value(1))
        };
        assert!(admitted(&caller(
            vec![ms(16, true, None), helper_call()],
            12,
            vec![]
        )));
        assert!(!admitted(&caller(
            vec![ms(8, true, None), helper_call()],
            12,
            vec![]
        )));
        assert!(!admitted(&caller(
            vec![ms(16, false, None), helper_call()],
            12,
            vec![]
        )));
        let used = caller(
            vec![ms(16, true, Some(50)), helper_call()],
            12,
            vec![Instruction::Store {
                val: Operand::Value(Value(50)),
                ptr: Value(100),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            }],
        );
        assert!(!admitted(&used));
    }

    #[test]
    fn const_branch_only_static_inline_admits_constant_arg_and_skips_variable_arg() {
        let mut f = IrFunction::new(
            "is_invalid_utf8_like".into(),
            IrType::I32,
            vec![crate::ir::reexports::IrParam {
                ty: IrType::I32,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: vec![],
                is_f128_sse: false,
                riscv_float_class: None,
            }],
            false,
        );
        f.is_static = true;
        f.is_inline = true;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::Switch {
                val: Operand::Value(Value(0)),
                cases: vec![(2, BlockId(1)), (3, BlockId(2))],
                default: BlockId(1),
                ty: IrType::I32,
            },
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            source_spans: vec![],
        });
        let mask = const_branch_param_mask(&f);
        assert_eq!(mask, 1);
        let p_sizes = vec![None];
        assert!(const_branch_site_arg(
            mask,
            &p_sizes,
            &[Operand::Const(IrConst::I32(3))]
        ));
        assert!(!const_branch_site_arg(
            mask,
            &p_sizes,
            &[Operand::Value(Value(9))]
        ));
    }
}

#[cfg(test)]
mod param_home_tests {
    //! Parameter homes are resolved through `param_alloca_values`, never by
    //! position (`inlined_param_homes`). Each case is a shape the positional
    //! "first N allocas" rule bridged through the wrong slot or ahead of the
    //! slot's definition; the IR verifier is the oracle.
    use super::*;
    use crate::ir::reexports::{
        BasicBlock, BlockId, CallInfo, Instruction, IrConst, IrFunction, IrModule, IrParam,
        Operand, Terminator, Value,
    };

    fn callee(name: &str, blocks: Vec<BasicBlock>, homes: Vec<Value>, next: u32) -> IrFunction {
        let mut f = IrFunction::new(
            name.to_string(),
            IrType::I32,
            vec![IrParam {
                ty: IrType::I32,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: Vec::new(),
                is_f128_sse: false,
                riscv_float_class: None,
            }],
            false,
        );
        f.is_static = true;
        f.is_always_inline = true;
        f.blocks = blocks;
        f.param_alloca_values = homes;
        f.next_value_id = next;
        f
    }

    fn caller_of(name: &str) -> IrFunction {
        let mut f = IrFunction::new("caller".to_string(), IrType::I32, vec![], false);
        f.next_value_id = 2;
        f.next_label = 101;
        f.blocks.push(BasicBlock {
            label: BlockId(100),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::Call {
                    func: name.to_string(),
                    info: CallInfo {
                        dest: Some(Value(1)),
                        args: vec![Operand::Value(Value(0))],
                        arg_types: vec![IrType::I32],
                        return_type: IrType::I32,
                        num_fixed_args: 1,
                        ..CallInfo::default()
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });
        f
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn alloca(dest: u32, ty: IrType, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty,
            size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn inline_into_caller(callee: IrFunction) -> IrModule {
        let name = callee.name.clone();
        let mut module = IrModule::new();
        module.functions = vec![caller_of(&name), callee];
        super::inline_run(&mut module, false);
        module
    }

    fn calls(f: &IrFunction, name: &str) -> bool {
        f.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func, .. } if func == name))
        })
    }

    fn violations(f: &IrFunction) -> Vec<String> {
        let mut out = Vec::new();
        crate::passes::verify::verify_function(f, "test", &mut out);
        out.into_iter().map(|v| v.detail).collect()
    }

    /// The `_mm_set1_pi32` shape after nested inlining: the parameter's dead
    /// home `v0` was deleted, and the first remaining alloca is an inner
    /// temporary defined after the `ParamRef`. Checked with lowering's record
    /// and without one (where only the leading alloca run may be homes).
    #[test]
    fn a_deleted_home_is_synthesized_not_taken_from_a_later_temporary() {
        for record in [vec![Value(0)], Vec::new()] {
            for param_value in [1, 115] {
                deleted_home_case(record.clone(), param_value);
            }
        }
    }

    fn deleted_home_case(record: Vec<Value>, param_value: u32) {
        let body = vec![
            Instruction::ParamRef {
                dest: Value(param_value),
                param_idx: 0,
                ty: IrType::I32,
            },
            alloca(2, IrType::Ptr, 8),
            Instruction::Store {
                val: Operand::Value(Value(param_value)),
                ptr: Value(2),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Load {
                dest: Value(3),
                ptr: Value(2),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ];
        let f = callee(
            "set1",
            vec![block(
                0,
                body,
                Terminator::Return(Some(Operand::Value(Value(3)))),
            )],
            record,
            4,
        );
        let module = inline_into_caller(f);
        let caller = &module.functions[0];
        assert!(!calls(caller, "set1"), "the call must still inline");
        assert_eq!(violations(caller), Vec::<String>::new());
        // The temporary (the only Ptr alloca) never receives the argument.
        let ptr_allocas: Vec<_> = caller
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| match i {
                Instruction::Alloca {
                    dest,
                    ty: IrType::Ptr,
                    ..
                } => Some(*dest),
                _ => None,
            })
            .collect();
        assert!(!ptr_allocas.is_empty(), "temporary survives");
        let stores_arg_into_temp = caller.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
            matches!(i, Instruction::Store { val: Operand::Value(Value(0)), ptr, .. } if ptr_allocas.contains(ptr))
        });
        assert!(!stores_arg_into_temp);
    }

    #[test]
    fn an_unrecorded_leading_same_typed_local_is_not_a_home() {
        let f = callee(
            "same_type_local",
            vec![block(
                0,
                vec![
                    alloca(0, IrType::I32, 4),
                    Instruction::ParamRef {
                        dest: Value(1),
                        param_idx: 0,
                        ty: IrType::I32,
                    },
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::Load {
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::BinOp {
                        dest: Value(3),
                        op: crate::ir::reexports::IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Value(Value(2)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            )],
            vec![],
            4,
        );
        let module = inline_into_caller(f);
        let caller = &module.functions[0];
        assert!(!calls(caller, "same_type_local"));
        assert_eq!(violations(caller), Vec::<String>::new());
        let allocas: Vec<_> = caller
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| match i {
                Instruction::Alloca { dest, .. } => Some(*dest),
                _ => None,
            })
            .collect();
        assert_eq!(
            allocas.len(),
            2,
            "the same-typed local and the fresh parameter home stay distinct"
        );
    }

    #[test]
    fn a_recorded_home_with_the_wrong_abi_shape_is_rejected() {
        let f = callee(
            "wrong_shape",
            vec![block(
                0,
                vec![
                    alloca(0, IrType::Ptr, 8),
                    Instruction::ParamRef {
                        dest: Value(1),
                        param_idx: 0,
                        ty: IrType::I32,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            vec![Value(0)],
            2,
        );
        let mut module = IrModule::new();
        module.functions.push(f);
        let map = build_callee_map(&module);
        let data = &map["wrong_shape"];
        assert!(inlined_param_homes(data, &data.blocks, 0).is_none());
    }

    /// A recorded home behind a non-alloca is hoisted ahead of the bridge.
    #[test]
    fn a_home_after_the_leading_alloca_run_is_hoisted() {
        let body = vec![
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty: IrType::I32,
            },
            alloca(0, IrType::I32, 4),
            Instruction::Store {
                val: Operand::Value(Value(1)),
                ptr: Value(0),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Load {
                dest: Value(2),
                ptr: Value(0),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ];
        let f = callee(
            "late_home",
            vec![block(
                0,
                body,
                Terminator::Return(Some(Operand::Value(Value(2)))),
            )],
            vec![Value(0)],
            3,
        );
        let module = inline_into_caller(f);
        let caller = &module.functions[0];
        assert!(!calls(caller, "late_home"));
        assert_eq!(violations(caller), Vec::<String>::new());
    }

    /// A recorded home outside the entry block cannot be dominated by the
    /// entry-block bridge: the site is refused, never miscompiled.
    #[test]
    fn a_home_outside_the_entry_block_refuses_the_site() {
        let entry = vec![Instruction::ParamRef {
            dest: Value(1),
            param_idx: 0,
            ty: IrType::I32,
        }];
        let next = vec![
            alloca(0, IrType::I32, 4),
            Instruction::Store {
                val: Operand::Value(Value(1)),
                ptr: Value(0),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Load {
                dest: Value(2),
                ptr: Value(0),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ];
        let f = callee(
            "far_home",
            vec![
                block(0, entry, Terminator::Branch(BlockId(1))),
                block(1, next, Terminator::Return(Some(Operand::Value(Value(2))))),
            ],
            vec![Value(0)],
            3,
        );
        let module = inline_into_caller(f);
        let caller = &module.functions[0];
        assert!(calls(caller, "far_home"), "the site must be refused");
        assert_eq!(violations(caller), Vec::<String>::new());
    }
}
