//! Linear-scan register allocator.
//!
//! GPR: (1) callee-saved for values live *across* a call (`start < cp < end`)
//! except cheap remats; (2) caller-saved for the rest (i686: per-reg scratch
//! filter); (3) leftover callee-saved for call-free overflow.
//!
//! Copy groups with pairwise-disjoint intervals share a home. Loop-carried
//! phi dests may steal colder callee-saved homes on AArch64, i686 and x86-64.
//! `detect_phi_coalesce_groups` is shared with stack-slot coalescing.

use super::live_range::{self, LinearScanAllocator};
use super::liveness::{
    LiveInterval, LivenessResult, compute_live_intervals, compute_live_intervals_with_hidden_reads,
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator};
use std::sync::Arc;

mod hot_web;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysReg(pub u8);

/// Single source of truth for the MachInst loop-size threshold.
///
/// This is a sanity bound for the SSA-driven window allocator, not a
/// profitability gate; keeping the constructor and tests on this constant
/// prevents a rebase from leaving one stale default literal behind.
pub(crate) const MI_MAX_LOOP_INSTS_DEFAULT: usize = 4096;

/// Immutable register-allocation/codegen policy captured once for a compiler
/// invocation.  `CCC_*` remains the public bisection surface; parsing it here
/// keeps mixed-polarity switches out of the per-function allocator hot path.
///
/// The field comments are the compatibility contract: every environment name,
/// its polarity, and its default live in this one declaration.  `NO_*` fields
/// default to `false`; positive diagnostic/experimental fields likewise default
/// to `false` unless a numeric or text default is stated explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RaConfig {
    // Core allocation / fusion policy.
    /// `CCC_NO_LEAF_PARAM_GPR`: disable leaf ABI parameter homes (default: false).
    pub(crate) no_leaf_param_gpr: bool,
    /// `CCC_NO_IR_DIVREM`: disable same-block div/rem pairing (default: false).
    pub(crate) no_ir_divrem: bool,
    /// `CCC_NO_MULACC`: disable i686 mul-accumulate pairing (default: false).
    pub(crate) no_mulacc: bool,
    /// `CCC_NO_INDEX_HOME`: disable folded-index homes (default: false).
    pub(crate) no_index_home: bool,
    /// `CCC_NO_FOLDED_INDEX_LIVENESS`: disable folded-index interval extension (default: false).
    pub(crate) no_folded_index_liveness: bool,
    /// `CCC_NO_FOLD_ADDR_DEFER`: keep constant-offset fold addresses as
    /// ordinary allocation candidates (default: false).
    pub(crate) no_fold_addr_defer: bool,
    /// `CCC_NO_VECREG`: disable vector-register allocation (default: false).
    pub(crate) no_vecreg: bool,
    /// `CCC_NO_PHI_COALESCE`: disable phi-copy coalescing (default: false).
    pub(crate) no_phi_coalesce: bool,
    /// `CCC_NO_COALESCE`: disable general copy-web coalescing (default: false).
    pub(crate) no_coalesce: bool,
    /// `CCC_NO_HOT_LOOP`: disable hot-loop Phase-1 homes (default: false).
    pub(crate) no_hot_loop: bool,
    /// `CCC_LEAF_STRICT_CALL_FREE`: require a truly call-free leaf policy (default: false).
    pub(crate) leaf_strict_call_free: bool,
    /// `CCC_NO_LEAF_CALLER_HOME`: disable volatile homes for eligible leaves (default: false).
    pub(crate) no_leaf_caller_home: bool,
    /// `CCC_DISABLE_PASSES=ipa_ra` (or `all`): disable interprocedural
    /// register allocation, i.e.
    /// caller-saved homes kept across calls of local callees that provably
    /// never write them (`x86::codegen::ipa_ra`; default: false).
    pub(crate) no_ipa_ra: bool,
    /// `CCC_NO_SPAN_VALVE`: disable the span-pressure valve globally, in
    /// every scan (default: false — the valve is on). Diagnostic/A/B
    /// switch only: the valve is load-bearing (chacha20_core goes from
    /// 33 to 91 spills with it off), so this must never become default.
    pub(crate) no_span_valve: bool,
    /// `CCC_NO_WEB_INLOOP_USE`: disable the web-wide in-loop-use supply for
    /// merged coalesce members in `mark_loop_spanning` (default: false — the
    /// supply is on). Diagnostic/A-B switch only: it is load-bearing
    /// (`sha256_transform` loses 3.6-4.3% runtime with it off, and the round
    /// loop's hottest slot goes back to 7 reloads per iteration), so this must
    /// never become default.
    pub(crate) no_web_inloop_use: bool,
    /// `CCC_RA_NO_RECURRENCE_FLOOR`: disable the recurrence floor at the
    /// no-victim demotion site (default: false — the floor is on). A loop
    /// recurrence with in-loop reads that ordinary eviction cannot serve
    /// steals the cheapest non-recurrence active instead of taking a slot;
    /// the floor is what keeps loop-carried cursors/counters in registers
    /// when function-spanning webs fill the pool (linux_rbtree's outer
    /// loop). Diagnostic/A-B switch only: load-bearing, never default off.
    pub(crate) no_recurrence_floor: bool,
    /// `CCC_NO_LOAD_HAZARD_REFINE`: disable i686 load hazard refinement (default: false).
    pub(crate) no_load_hazard_refine: bool,
    /// `CCC_NO_EAX_ALLOC`: disable the i686 eax allocation phase (default: false).
    pub(crate) no_eax_alloc: bool,
    /// `CCC_NO_LOOP_PIN`: disable AArch64 loop-pin steals (default: false).
    pub(crate) no_loop_pin: bool,
    /// `CCC_LOOP_PIN`: maximum AArch64 loop-pin steals (default: 2).
    pub(crate) loop_pin: usize,
    /// `CCC_NO_HOT_WEB_STEAL`: disable i686/x86-64 hot-web steals (default: false).
    pub(crate) no_hot_web_steal: bool,
    /// `CCC_NO_P1_WAVES`: merge the two Phase-1 waves into the historical
    /// single scan (default: false).  Bisection switch for the mandatory
    /// (call-spanning) before optional (hot-loop) allocation order.
    pub(crate) no_p1_waves: bool,
    /// `CCC_HOT_WEB_STEAL`: maximum i686/x86-64 hot-web steals (default: 3).
    pub(crate) hot_web_steal: usize,
    /// `CCC_NO_ITERATED_HAZARD`: disable i686 iterated hazard refinement (default: false).
    pub(crate) no_iterated_hazard: bool,
    /// `CCC_NO_SEGMENT_FILL`: disable segment-fill allocation (default: false).
    pub(crate) no_segment_fill: bool,
    /// `CCC_SEGMENT_FILL_LIMIT=N`: bisection aid — admit at most N Phase-2f
    /// segment fills per allocation run (default: unlimited).  Combine with
    /// `CCC_SEGMENT_FILL_FUNC` to confine the limit to one function; the
    /// `CCC_DEBUG_SEGMENT_FILL` trace lists the fills in admission order.
    pub(crate) segment_fill_limit: Option<usize>,
    /// `CCC_SEGMENT_FILL_FUNC=name`: restrict `CCC_SEGMENT_FILL_LIMIT` to the
    /// named function (default: the limit, when set, applies everywhere).
    pub(crate) segment_fill_func: Option<String>,
    /// `CCC_NO_REDUCTION_VECREG`: disable reduction vector homes (default: false).
    pub(crate) no_reduction_vecreg: bool,
    /// `CCC_NO_MAP_VECREG`: disable map vector homes (default: false).
    pub(crate) no_map_vecreg: bool,
    /// `CCC_NO_FP_COPY_WEB`: disable x86 FP copy-web homes (default: false).
    pub(crate) no_fp_copy_web: bool,
    /// `CCC_FP_EXTRACT_HOMES`: experimental scalar lane-extract homes (default: false).
    /// Correctness/assembly validated; runtime is highly link-layout-sensitive.
    /// Keep opt-in until layout-distributed and target-hardware A/B is stable.
    pub(crate) fp_extract_homes: bool,
    /// `CCC_CALLER_SAVE_SPANNING`: enable caller-save spans (default: false).
    pub(crate) caller_save_spanning: bool,
    /// `CCC_NO_COLD_CALL_SPLIT`: keep scalar FP values live across cold calls
    /// out of the x86-64 XMM pool (default: false).
    pub(crate) no_cold_call_split: bool,
    /// `CCC_NO_PHI_BLOCKER_REHOME`: never move the values blocking a loop
    /// phi coalesce into a free callee-saved home (default: false).
    pub(crate) no_phi_blocker_rehome: bool,
    /// `CCC_NO_PHASE2C_REUSE`: skip Phase 2c's second tier, which homes what
    /// the fresh callee-saved scan left over in the free stretches of
    /// already-saved callee-saved registers (default: false).
    pub(crate) no_phase2c_reuse: bool,
    /// `CCC_NO_CALLEE_STICKY`: the Phase 1 callee-saved scan rotates through
    /// fresh registers instead of reusing touched ones (default: false).
    pub(crate) no_callee_sticky: bool,
    /// `CCC_NO_SAVE_SET_COMPACT`: keep every callee-saved register the
    /// phases chose instead of folding a register's whole holder set into
    /// another already-saved one (default: false).
    pub(crate) no_save_set_compact: bool,
    /// `CCC_NO_RDX_HAZARD`: disable position-aware %rdx admission on x86-64
    /// (the Phase-2 hazard-filtered %rdx wave) and fall back to the
    /// whole-function exclusion whenever any body instruction clobbers %rdx
    /// (default: false — the wave is on).
    pub(crate) no_rdx_hazard: bool,
    /// `CCC_NO_RDX_PARAM_HOMES`: restore the whole-function rule that any
    /// %rdx-clobbering instruction keeps every x86-64 parameter out of the
    /// caller-saved pool (default: false -- division/multiply-high/switch
    /// bodies grant parameters caller-saved homes, see
    /// [`x86_rdx_clobbers_allow_param_homes`]).
    pub(crate) no_rdx_param_homes: bool,
    /// `CCC_NO_SEGMENT_SCAN`: disable hole-aware scanning (default: false).
    pub(crate) no_segment_scan: bool,
    /// `CCC_EVICT_MODE`: eviction mode (default: 3; malformed values use 3).
    pub(crate) evict_mode: i32,
    /// `CCC_EVICT_SHORT_K`: cost-ratio escape for the mode>=3 next-use
    /// gate (default: 16; 0 disables the escape). A short victim whose
    /// next use dies before the incoming ends is still evicted when the
    /// incoming outweighs it by more than Kx. Malformed values use 16.
    pub(crate) evict_short_k: u64,
    /// `CCC_RA_LOOP_SPAN_RESERVE`: CEILING on the registers the main scan
    /// reserves for block-local ranges by capping how many registers
    /// loop-spanning ranges may hold at once (default: 0 = OFF). The
    /// effective reserve per loop is that loop's measured peak block-local
    /// range concurrency, clamped to this ceiling. Armed with 3 the cap
    /// measured chacha20 +29% and sha256 +27% (ARX loops: many single-use
    /// temporaries starved by loop-spanning webs) with arith_loop -10%
    /// (once-per-pass recurrence slots must not be demoted) and scattered
    /// -3..-5%; the net geomean was inside the noise band, so the default
    /// stays off until a stable-machine census (and the calibration noted
    /// in the RA-PRESSURE-1 follow-up) ratifies a default. RECALIBRATION
    /// (2026-09-08, S03 supreme tree): the +29% does NOT replicate here —
    /// reserve=3 on chacha20_block at -O2 measures 1.626x vs 1.603x
    /// default (noise; the knob engages — 313 asm lines differ — and the
    /// mov gap persists at 232 vs GCC's 90, so the cap optimizes the
    /// wrong pressure on this tree). Default stays off; the census must
    /// re-measure from this tree, not from the S02 numbers. CENSUS
    /// (2026-10-22, reserve=4 vs default, paired in one runner invocation,
    /// 9 reps / 2 warmups, output-checked, 49 benchmarks): net geomean
    /// 1.0004 over all 49 and 0.9970 over the 31 whose median runtime
    /// clears 20 ms — still inside the noise band, so the default stays
    /// off. The shape split is consistent and reproducible: wins on
    /// pointer-chasing code (linux_rbtree 1.190x, lz4_match_extend 1.061x,
    /// glibc_strstr 1.030x) against losses on recurrence/ARX code
    /// (expat_siphash24 0.912x, hash_table 0.954x, sha256_transform 0.976x,
    /// fannkuch 0.979x), the same split the arith_loop note describes. The
    /// mechanism is real where it wins: on linux_rbtree the cap is what lets
    /// the tree-descent phi keep a register instead of being materialized
    /// through a stack slot (store in each arm, reload at the join, once per
    /// descent step), taking main's stack references from 20 to 15 against
    /// GCC's 0. So a blanket ceiling is the wrong instrument — the reserve
    /// has to be conditioned on the loop's shape, not applied uniformly.
    /// The knob also
    /// arms the GEP-base and coalesce-web cost corrections in the main
    /// waves, so cap-off is byte-identical to the pre-cap allocator.
    /// Rationale (RA-PRESSURE-1): a whole-range linear scan assigns
    /// loop-spanning webs first (earliest start), and a single-use
    /// block-local temp can never outbid a web's remaining-use cost, so
    /// once the webs saturate the pool every temp in the loop is staged
    /// through memory (chacha20: 178 movs vs GCC's 65, all ~60 QR temps
    /// slot-homed). The cap enforces the pigeonhole up front: when there
    /// are more webs than pool−K, the excess webs spill at admission and
    /// K registers stay available for the temps, which expire and free
    /// them again within a few instructions.
    pub(crate) loop_span_reserve: usize,
    /// `CCC_PGO_WEIGHT_MAX`: PGO multiplier cap (default: 1; clamped to 1..=16).
    pub(crate) pgo_weight_max: u64,

    // Allocation diagnostics and scoped bisection filters.
    /// `CCC_DEBUG_COALESCE`: emit copy-web diagnostics (default: false).
    pub(crate) debug_coalesce: bool,
    /// `CCC_DEBUG_COALESCE_MEMBERS`: emit copy-web member diagnostics (default: false).
    pub(crate) debug_coalesce_members: bool,
    /// `CCC_DEBUG_PHI_COALESCE`: emit phi-copy diagnostics (default: false).
    pub(crate) debug_phi_coalesce: bool,
    /// `CCC_DEBUG_RA_PHASES`: emit allocator phase diagnostics (default: false).
    pub(crate) debug_ra_phases: bool,
    /// `CCC_DEBUG_RA_INTERVALS`: emit allocator interval diagnostics (default: false).
    pub(crate) debug_ra_intervals: bool,
    /// `CCC_DEBUG_SEGMENT_FILL`: emit segment-fill diagnostics (default: false).
    pub(crate) debug_segment_fill: bool,
    /// `CCC_DEBUG_RA`: emit general allocator diagnostics (default: false).
    pub(crate) debug_ra: bool,
    /// `CCC_DEBUG_RA_REPAIR`: emit allocator repair diagnostics (default: false).
    pub(crate) debug_ra_repair: bool,
    /// `CCC_DEBUG_HAZARDS`: emit i686 hazard diagnostics (default: false).
    pub(crate) debug_hazards: bool,
    /// `CCC_TRACE_ALLOC`: emit linear-scan assignment tracing (default: false).
    pub(crate) trace_alloc: bool,
    /// `CCC_TRACE_ALLOCSTATS`: enable allocation statistics (default: false).
    pub(crate) trace_allocstats: bool,
    /// Text value of `CCC_TRACE_ALLOCSTATS` (default: absent).
    pub(crate) trace_allocstats_filter: Option<String>,
    /// `CCC_VERIFY_REGALLOC`: verify full allocation history (default: false).
    pub(crate) verify_regalloc: bool,
    /// `LCCC_DBG_RA`: enable legacy allocator diagnostics (default: false).
    pub(crate) legacy_debug_ra: bool,
    /// `LCCC_DBG_RA_FUNC`: legacy exact function filter (default: empty).
    pub(crate) legacy_debug_ra_func: String,
    /// `CCC_RA_EXPLAIN`: allocation-explanation function filter (default: absent).
    pub(crate) ra_explain: Option<String>,
    /// `CCC_RA_EXPLAIN_HOMES`: include homes in allocation explanations (default: false).
    pub(crate) ra_explain_homes: bool,
    /// `CCC_RA_DROP`: comma-separated values forced out of allocation (default: absent).
    pub(crate) ra_drop: Option<String>,
    /// `CCC_RA_DROP_FUNC`: scope `CCC_RA_DROP` to a function (default: absent).
    pub(crate) ra_drop_func: Option<String>,
    /// `CCC_PHI_COALESCE_SKIP`: comma-separated phi sources to preserve (default: absent).
    pub(crate) phi_coalesce_skip: Option<String>,
    /// `CCC_PHI_COALESCE_FUNC`: function scope for phi skip list (default: absent).
    pub(crate) phi_coalesce_func: Option<String>,

    // Shared helper policy.
    /// `CCC_NO_ABI_REG_HINTS`: disable ABI register hints (default: false).
    pub(crate) no_abi_reg_hints: bool,
    /// `CCC_DISABLE_SCALAR_FP_XMM`: request scalar-FP XMM home disable (default: false).
    pub(crate) disable_scalar_fp_xmm: bool,
    /// `CCC_ENABLE_SCALAR_FP_XMM`: override `CCC_DISABLE_SCALAR_FP_XMM` (default: false).
    pub(crate) enable_scalar_fp_xmm: bool,
    /// `CCC_NO_XMM_REGALLOC`: disable scalar-FP XMM allocation (default: false).
    pub(crate) no_xmm_regalloc: bool,
    /// `CCC_NO_PROMOTED_FP_TAIL`: reserve the promoted FP tail (default: false).
    pub(crate) no_promoted_fp_tail: bool,
    /// `CCC_NO_FP_CALLEE_SAVED`: exclude AArch64 callee-saved FP homes (default: false).
    pub(crate) no_fp_callee_saved: bool,
    /// `CCC_NO_REGALLOC`: force slot-only allocation (default: false).
    pub(crate) no_regalloc: bool,
    /// `CCC_NO_REGALLOC_FUNC`: comma-separated function names forced slot-only (default: absent).
    pub(crate) no_regalloc_func: Option<String>,

    // x86 prologue / MachInst policy.
    /// `CCC_DUMP_IR`: dump IR in the pass and codegen diagnostics (default: false).
    pub(crate) dump_ir: bool,
    /// `CCC_DUMP_IR_FUNC`: substring filter for codegen IR dumps (default: absent).
    pub(crate) dump_ir_func: Option<String>,
    /// `CCC_NO_VA_ROOT_GUARD`: disable conservative va_list root guarding (default: false).
    pub(crate) no_va_root_guard: bool,
    /// `CCC_DEBUG_VARARG`: emit vararg classification diagnostics (default: false).
    pub(crate) debug_vararg: bool,
    /// `CCC_NO_X64_IMMED_NOHOME`: disable x86 immediate-consumer no-home policy (default: false).
    pub(crate) no_x64_immed_nohome: bool,
    /// `CCC_NO_I686_ACCUM_NOHOME`: disable the i686 "immediately-consumed
    /// values get no home" policy (default: false).  The i686 twin of
    /// `CCC_NO_X64_IMMED_NOHOME`; it exists because the policy's premise —
    /// a register home only ever costs an extra `movl %R,%eax` relay — is
    /// false for the ops `try_emit_int_binop_direct` handles, which read a
    /// register-homed operand IN PLACE.  Keep it a switch: the boot corpus
    /// A/B in engineering/journal decides the default, not the theory.
    pub(crate) no_i686_accum_nohome: bool,
    /// `CCC_NO_X64_PUSH_ARG_FORMS`: disable the x86-64 direct stack-argument
    /// push forms (default: false).  With the switch off, a MEMORY-class
    /// scalar argument stages as `pushq $imm` / `pushq %reg` /
    /// `pushq slot(%rsp)` — each strictly smaller than the fallback
    /// materialise-into-%rax + `pushq %rax`, and each removes %rax from the
    /// staging serialisation chain.  Kill switch for bisection, per the
    /// house rule that every staging change is individually measurable.
    pub(crate) no_x64_push_arg_forms: bool,
    /// `CCC_X64_NOHOME_CLASSES`: selected no-home consumer classes (default: `ret,store,copy,cast,unary,binop`).
    pub(crate) x64_nohome_classes: String,
    /// `CCC_MI_MAX_LOOP_INSTS`: MachInst loop-size threshold
    /// (default: [`MI_MAX_LOOP_INSTS_DEFAULT`]).
    pub(crate) mi_max_loop_insts: usize,
    /// `CCC_MI_FN_DISABLE`: comma-separated MachInst-disabled name substrings (default: empty).
    pub(crate) mi_fn_disable: String,
    /// `CCC_MI_ALL_CLASSIC`: suppress normal MachInst per-function use (default: false).
    pub(crate) mi_all_classic: bool,
    /// `CCC_MI_FN_FORCE`: comma-separated MachInst-forced name substrings (default: empty).
    pub(crate) mi_fn_force: String,
    /// `CCC_MI_FORCE_LOOPS`: bypass the MachInst loop-size threshold (default: false).
    pub(crate) mi_force_loops: bool,
    /// `CCC_MI_DEBUG`: emit MachInst profitability diagnostics (default: false).
    pub(crate) mi_debug: bool,
    /// `CCC_NO_LOAD_CAST_FOLD`: disable x86 load-cast folding (default: false).
    pub(crate) no_load_cast_fold: bool,
    /// `CCC_DEBUG_LOAD_CAST_FOLD`: emit x86 load-cast diagnostics (default: false).
    pub(crate) debug_load_cast_fold: bool,
    /// `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`: retain empty x86 local frames (default: false).
    pub(crate) no_empty_local_frame_elision: bool,
    /// `CCC_DEBUG_PARAM_STORE`: emit parameter-store diagnostics (default: false).
    pub(crate) debug_param_store: bool,
    /// `CCC_DEBUG_PARAMREF`: emit parameter-reference diagnostics (default: false).
    pub(crate) debug_paramref: bool,
    /// `CCC_NO_MACHINST`: disable MachInst globally (default: false).
    pub(crate) no_machinst: bool,
    /// `CCC_MI_DISABLE_KINDS`: comma-separated MachInst kind mask (default: empty).
    pub(crate) mi_disable_kinds: String,
}

impl Default for RaConfig {
    fn default() -> Self {
        Self::from_sources(|_| false, |_| None)
    }
}

impl RaConfig {
    /// Capture all compatibility knobs once, at compiler-invocation setup.
    pub(crate) fn from_process_env() -> Self {
        Self::from_sources(
            |name| std::env::var_os(name).is_some(),
            |name| std::env::var(name).ok(),
        )
    }

    /// Factored solely for table-driven parser tests. `present` mirrors
    /// `var_os(...).is_some()` for boolean switches, while `text` mirrors
    /// `var(...).ok()` for values and keeps their historical UTF-8 behavior.
    fn from_sources<P, T>(present: P, text: T) -> Self
    where
        P: Fn(&str) -> bool,
        T: Fn(&str) -> Option<String>,
    {
        let number = |name: &str, default: usize| {
            text(name)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(default)
        };
        Self {
            no_leaf_param_gpr: present("CCC_NO_LEAF_PARAM_GPR"),
            no_ir_divrem: present("CCC_NO_IR_DIVREM"),
            no_mulacc: present("CCC_NO_MULACC"),
            no_index_home: present("CCC_NO_INDEX_HOME"),
            no_folded_index_liveness: present("CCC_NO_FOLDED_INDEX_LIVENESS"),
            no_fold_addr_defer: present("CCC_NO_FOLD_ADDR_DEFER"),
            no_vecreg: present("CCC_NO_VECREG"),
            no_phi_coalesce: present("CCC_NO_PHI_COALESCE"),
            no_coalesce: present("CCC_NO_COALESCE"),
            no_hot_loop: present("CCC_NO_HOT_LOOP"),
            leaf_strict_call_free: present("CCC_LEAF_STRICT_CALL_FREE"),
            no_leaf_caller_home: present("CCC_NO_LEAF_CALLER_HOME"),
            no_ipa_ra: text("CCC_DISABLE_PASSES")
                .is_some_and(|list| crate::passes::pass_disabled(list, "ipa_ra")),
            no_span_valve: present("CCC_NO_SPAN_VALVE"),
            no_web_inloop_use: present("CCC_NO_WEB_INLOOP_USE"),
            no_recurrence_floor: present("CCC_RA_NO_RECURRENCE_FLOOR"),
            no_load_hazard_refine: present("CCC_NO_LOAD_HAZARD_REFINE"),
            no_eax_alloc: present("CCC_NO_EAX_ALLOC"),
            no_loop_pin: present("CCC_NO_LOOP_PIN"),
            loop_pin: number("CCC_LOOP_PIN", 2),
            no_hot_web_steal: present("CCC_NO_HOT_WEB_STEAL"),
            no_p1_waves: present("CCC_NO_P1_WAVES"),
            hot_web_steal: number("CCC_HOT_WEB_STEAL", 3),
            no_iterated_hazard: present("CCC_NO_ITERATED_HAZARD"),
            no_segment_fill: present("CCC_NO_SEGMENT_FILL"),
            segment_fill_limit: text("CCC_SEGMENT_FILL_LIMIT")
                .and_then(|value| value.parse::<usize>().ok()),
            segment_fill_func: text("CCC_SEGMENT_FILL_FUNC"),
            no_reduction_vecreg: present("CCC_NO_REDUCTION_VECREG"),
            no_map_vecreg: present("CCC_NO_MAP_VECREG"),
            no_fp_copy_web: present("CCC_NO_FP_COPY_WEB"),
            fp_extract_homes: present("CCC_FP_EXTRACT_HOMES"),
            caller_save_spanning: present("CCC_CALLER_SAVE_SPANNING"),
            no_cold_call_split: present("CCC_NO_COLD_CALL_SPLIT"),
            no_phi_blocker_rehome: present("CCC_NO_PHI_BLOCKER_REHOME"),
            no_phase2c_reuse: present("CCC_NO_PHASE2C_REUSE"),
            no_callee_sticky: present("CCC_NO_CALLEE_STICKY"),
            no_save_set_compact: present("CCC_NO_SAVE_SET_COMPACT"),
            no_rdx_hazard: present("CCC_NO_RDX_HAZARD"),
            no_rdx_param_homes: present("CCC_NO_RDX_PARAM_HOMES"),
            no_segment_scan: present("CCC_NO_SEGMENT_SCAN"),
            evict_mode: text("CCC_EVICT_MODE")
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(3),
            evict_short_k: text("CCC_EVICT_SHORT_K")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(16),
            loop_span_reserve: number("CCC_RA_LOOP_SPAN_RESERVE", 0),
            pgo_weight_max: text("CCC_PGO_WEIGHT_MAX")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1)
                .clamp(1, 16),

            debug_coalesce: present("CCC_DEBUG_COALESCE"),
            debug_coalesce_members: present("CCC_DEBUG_COALESCE_MEMBERS"),
            debug_phi_coalesce: present("CCC_DEBUG_PHI_COALESCE"),
            debug_ra_phases: present("CCC_DEBUG_RA_PHASES"),
            debug_ra_intervals: present("CCC_DEBUG_RA_INTERVALS"),
            debug_segment_fill: present("CCC_DEBUG_SEGMENT_FILL"),
            debug_ra: present("CCC_DEBUG_RA"),
            debug_ra_repair: present("CCC_DEBUG_RA_REPAIR"),
            debug_hazards: present("CCC_DEBUG_HAZARDS"),
            trace_alloc: present("CCC_TRACE_ALLOC"),
            trace_allocstats: present("CCC_TRACE_ALLOCSTATS"),
            trace_allocstats_filter: text("CCC_TRACE_ALLOCSTATS"),
            verify_regalloc: present("CCC_VERIFY_REGALLOC"),
            legacy_debug_ra: present("LCCC_DBG_RA"),
            legacy_debug_ra_func: text("LCCC_DBG_RA_FUNC").unwrap_or_default(),
            ra_explain: text("CCC_RA_EXPLAIN"),
            ra_explain_homes: present("CCC_RA_EXPLAIN_HOMES"),
            ra_drop: text("CCC_RA_DROP"),
            ra_drop_func: text("CCC_RA_DROP_FUNC"),
            phi_coalesce_skip: text("CCC_PHI_COALESCE_SKIP"),
            phi_coalesce_func: text("CCC_PHI_COALESCE_FUNC"),

            no_abi_reg_hints: present("CCC_NO_ABI_REG_HINTS"),
            disable_scalar_fp_xmm: present("CCC_DISABLE_SCALAR_FP_XMM"),
            enable_scalar_fp_xmm: present("CCC_ENABLE_SCALAR_FP_XMM"),
            no_xmm_regalloc: present("CCC_NO_XMM_REGALLOC"),
            no_promoted_fp_tail: present("CCC_NO_PROMOTED_FP_TAIL"),
            no_fp_callee_saved: present("CCC_NO_FP_CALLEE_SAVED"),
            no_regalloc: present("CCC_NO_REGALLOC"),
            no_regalloc_func: text("CCC_NO_REGALLOC_FUNC"),

            dump_ir: present("CCC_DUMP_IR"),
            dump_ir_func: text("CCC_DUMP_IR_FUNC"),
            no_va_root_guard: present("CCC_NO_VA_ROOT_GUARD"),
            debug_vararg: present("CCC_DEBUG_VARARG"),
            no_x64_immed_nohome: present("CCC_NO_X64_IMMED_NOHOME"),
            no_i686_accum_nohome: present("CCC_NO_I686_ACCUM_NOHOME"),
            no_x64_push_arg_forms: present("CCC_NO_X64_PUSH_ARG_FORMS"),
            x64_nohome_classes: text("CCC_X64_NOHOME_CLASSES")
                .unwrap_or_else(|| "ret,store,copy,cast,unary,binop".into()),
            mi_max_loop_insts: number("CCC_MI_MAX_LOOP_INSTS", MI_MAX_LOOP_INSTS_DEFAULT),
            mi_fn_disable: text("CCC_MI_FN_DISABLE").unwrap_or_default(),
            mi_all_classic: present("CCC_MI_ALL_CLASSIC"),
            mi_fn_force: text("CCC_MI_FN_FORCE").unwrap_or_default(),
            mi_force_loops: present("CCC_MI_FORCE_LOOPS"),
            mi_debug: present("CCC_MI_DEBUG"),
            no_load_cast_fold: present("CCC_NO_LOAD_CAST_FOLD"),
            debug_load_cast_fold: present("CCC_DEBUG_LOAD_CAST_FOLD"),
            no_empty_local_frame_elision: present("CCC_NO_EMPTY_LOCAL_FRAME_ELISION"),
            debug_param_store: present("CCC_DEBUG_PARAM_STORE"),
            debug_paramref: present("CCC_DEBUG_PARAMREF"),
            no_machinst: present("CCC_NO_MACHINST"),
            mi_disable_kinds: text("CCC_MI_DISABLE_KINDS").unwrap_or_default(),
        }
    }
}

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
/// Compatibility helper for isolated unit tests. Production code must pass
/// the invocation-owned [`RaConfig`] through
/// [`analyze_accumulator_assignments_with_config`].
pub fn analyze_accumulator_assignments(
    func: &IrFunction,
    policy: AccumulatorPolicy,
) -> Vec<AccumulatorAssignment> {
    analyze_accumulator_assignments_with_config(func, policy, &RaConfig::default())
}

pub(crate) fn analyze_accumulator_assignments_with_config(
    func: &IrFunction,
    policy: AccumulatorPolicy,
    config: &RaConfig,
) -> Vec<AccumulatorAssignment> {
    let lhs_first = matches!(policy.operand_order, AccumulatorOperandOrder::LhsFirst);
    let mut candidates =
        crate::backend::stack_layout::copy_coalescing::compute_immediately_consumed(
            func, lhs_first,
        );
    if candidates.is_empty() {
        return Vec::new();
    }
    // An accumulator home is one register: a value proven wider than it (an
    // 8-byte I64/U64/F64/D64 pair on a 32-bit target, a 16-byte value
    // anywhere) would be consumed with a stale upper half. The candidate scan
    // types values by their producer, and a Copy has none of its own, so a
    // copy of a wide phi, constant, call result or parameter slipped through.
    // The fixpoint value-type map types every Copy/Phi web and consuming
    // context; untyped values keep the historical GPR treatment.
    let accumulator_bytes = if crate::common::types::target_is_32bit() {
        4
    } else {
        8
    };
    let value_types = crate::backend::common::compute_value_type_map(func);
    candidates.retain(|id| {
        value_types
            .get(id)
            .is_none_or(|ty| ty.size() <= accumulator_bytes)
    });
    if candidates.is_empty() {
        return Vec::new();
    }

    // Fused div/rem pair tails emit no code — their dest is NOT staged into
    // the accumulator at its IR def point (it was stored by the head). An
    // accumulator chain starting from a tail dest would let the consumer
    // read a stale %eax.
    let divrem = match divrem_target_for_current_arch() {
        Some(t) => compute_i686_divrem_pairs_with_config(func, t, config),
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
    // 64-bit targets do not consume scalar returns from the accumulator.
    // Collected once: a per-candidate scan of every terminator was
    // O(candidates x blocks) (gcc.c-torture/compile 20001226-1).
    let returned: FxHashSet<u32> = if policy.return_consumes_accumulator {
        FxHashSet::default()
    } else {
        func.blocks
            .iter()
            .filter_map(|b| match &b.terminator {
                Terminator::Return(Some(Operand::Value(v))) => Some(v.0),
                _ => None,
            })
            .collect()
    };
    let mut out = Vec::new();
    for value_id in candidates {
        let (Some(&def_point), Some(points)) = (defs.get(&value_id), uses.get(&value_id)) else {
            continue;
        };
        if points.len() != 1 || points[0] != def_point + 1 {
            continue;
        }
        if returned.contains(&value_id) {
            continue;
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
    /// The `RegAllocConfig::ipa_regs` the IPA-RA fill homed values in.
    pub ipa_used: Vec<PhysReg>,
    pub accumulator_assignments: Vec<AccumulatorAssignment>,
    pub used_regs: Vec<PhysReg>,
    pub caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>>,
    pub liveness: Option<LivenessResult>,
    /// Coalesced same-value classes: member value id → class representative
    /// (the minimum id, for determinism). Covers exactly the classes the
    /// overlap verifier blesses — slot coalescing webs, applied phi-copy
    /// destructive updates, and FP webs — because those are the groups whose
    /// members provably denote the SAME value at every program point where
    /// both are live. The x86 emitter's home-freshness bookkeeping tracks
    /// freshness per SSA id; an in-place chain update (`cmovnel %r10d,%r13d`
    /// for the newest id of a coalesced induction chain) legitimately evicts
    /// the OLDER member ids while the register content is still exactly
    /// their value (kernel 6.18.52 workqueue.c llc_populate_cpu_shard_id:
    /// v107→v109→v103 all homed r13 slot-less, the cmov freshened v109 only,
    /// and the consumer of v103 hit the fail-closed operand_to_rax gate with
    /// no slot to reload from). Read-side consumers consult this map to
    /// accept a FRESH class sibling as proof the home is readable. Empty on
    /// targets whose emitters are not alias-aware (ARM/RISC-V/i686 keep the
    /// pre-alias behavior).
    pub phi_chain: FxHashMap<u32, u32>,
    /// Values the allocator considered for a register home — the causal
    /// denominator of a spill.  A slot holding an *eligible* value that ended
    /// up with no register is register-pressure traffic; a slot holding a value
    /// that was never a candidate (alloca, address-taken, i128/vector/aggregate)
    /// is structurally required memory.  Only the allocator knows which is
    /// which, so it publishes the set for the census
    /// (`CCC_SLOT_CENSUS=1`, `scripts/stack_census.py`) instead of every
    /// consumer re-deriving eligibility and drifting from it.
    pub eligible: FxHashSet<u32>,
    /// Base -> constant-offset fold addresses deferred in this round (see
    /// `defer_const_fold_addresses`); the caller re-admits the addresses of
    /// every base that did not end in `RegAllocConfig::fold_base_regs`.
    pub fold_deferred: FxHashMap<u32, Vec<u32>>,
}

/// Whether x86 can preserve incoming parameters directly in caller-saved homes.
/// ParamRefs must execute in the entry prefix before any generated instruction
/// can clobber ABI argument registers, and the function must be call-free.
/// RISC-V analog of [`x86_param_caller_homes_safe`]: integer ABI slot i is
/// register a_i, so a call-free function whose ParamRefs all run in the
/// entry prefix (before any generated instruction can clobber a0–a7) can
/// keep its scalar parameters directly in their incoming registers — the
/// ParamRef emits nothing at all when the home matches the incoming reg.
/// Compatibility query for isolated callers. Production code must use the
/// explicit invocation policy variant below.
pub fn riscv_param_caller_homes_safe(func: &IrFunction) -> bool {
    riscv_param_caller_homes_safe_with_config(func, &RaConfig::default())
}

pub(crate) fn riscv_param_caller_homes_safe_with_config(
    func: &IrFunction,
    ra_config: &RaConfig,
) -> bool {
    if func.blocks.is_empty() || ra_config.no_leaf_param_gpr {
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

/// True when ANY instruction of `func` implicitly clobbers %rdx: division
/// (cqto/cltd sign-extends into rdx:rax), i128 arithmetic (the rax:rdx
/// pair), Switch (jump-table dispatch scratch), or a fixed-scratch
/// intrinsic. This is the SAME condition under which the x86-64 prologue
/// excludes %rdx (PhysReg 16) from allocation — both gates must stay in
/// lockstep: a value homed in %rdx is unsound exactly when the body can
/// overwrite %rdx behind the allocator's back.
/// Fixed-GPR scratch model of the x86-64 text emitters (single source of
/// truth — the prologue register census and the caller-saved-parameter-home
/// gate both consume it; the two used to keep hand-maintained copies and
/// drifted apart, C4: `Load { ty: I128 }` was missing from one of them).
///
/// `rdx`: the instruction's emitter writes `%rdx` behind the allocator's
/// back (division `cqto`/`idiv`, i128 `rax:rdx` pairs, `rdtsc(p)`,
/// jump-table dispatch, the cmpxchg-loop RMWs' candidate, cmpxchg's
/// `desired` operand, atomic stores, fixed-scratch vector intrinsics).
/// `rdi`: `rdtscp`, and the 128-bit atomics' libatomic argument staging.
/// The narrow cmpxchg-loop RMWs read their operand in place
/// (`emit_x86_atomic_op_loop`; stack-staged when it has no usable home), so
/// they no longer touch `%rdi`.
///
/// Values homed in a clobbered register across such an instruction read
/// garbage afterwards (at1: `__atomic_fetch_and` loop destroyed the `%rdx`
/// home of a live temp, at2: `cmpxchg` destroyed the `%rdx` home of a
/// parameter; both silent wrong results at -O1+).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct X86FixedScratch {
    pub rdx: bool,
    pub rdi: bool,
}

impl X86FixedScratch {
    #[inline]
    pub fn any(self) -> bool {
        self.rdx || self.rdi
    }
    #[inline]
    fn or(self, o: Self) -> Self {
        Self {
            rdx: self.rdx || o.rdx,
            rdi: self.rdi || o.rdi,
        }
    }
}

/// Fixed-GPR clobbers of ONE instruction's x86-64 emitter. See
/// [`X86FixedScratch`].
pub fn x86_inst_fixed_scratch(inst: &Instruction) -> X86FixedScratch {
    use crate::ir::ops::AtomicRmwOp;
    const RDX: X86FixedScratch = X86FixedScratch {
        rdx: true,
        rdi: false,
    };
    const RDX_RDI: X86FixedScratch = X86FixedScratch {
        rdx: true,
        rdi: true,
    };
    const NONE: X86FixedScratch = X86FixedScratch {
        rdx: false,
        rdi: false,
    };
    let wide = |t: &IrType| matches!(t, IrType::I128 | IrType::U128);
    match inst {
        Instruction::BinOp { op, ty, .. } => {
            if matches!(
                op,
                IrBinOp::SDiv
                    | IrBinOp::UDiv
                    | IrBinOp::SRem
                    | IrBinOp::URem
                    | IrBinOp::UMulHigh
                    | IrBinOp::SMulHigh
            ) || wide(ty)
            {
                RDX
            } else {
                NONE
            }
        }
        Instruction::UnaryOp { ty, .. }
        | Instruction::Cmp { ty, .. }
        | Instruction::Load { ty, .. } => {
            if wide(ty) {
                RDX
            } else {
                NONE
            }
        }
        // Store: wide values pair-stage through %rax:%rdx, and F128 stores'
        // non-direct-slot arms (`emit_f128_store_f64_via_x87`) stage the
        // value through `movq %rax, %rdx` (f128.rs OverAligned/Indirect/Reg
        // arms) before the x87 sequence.
        Instruction::Store { ty, .. } => {
            if wide(ty) || matches!(ty, IrType::F128) {
                RDX
            } else {
                NONE
            }
        }
        Instruction::Cast { from_ty, to_ty, .. } => {
            if wide(from_ty) || wide(to_ty) {
                RDX
            } else {
                NONE
            }
        }
        // `lock xadd` / `xchg` / test-and-set run on rax+rcx only; bitwise
        // RMWs use a cmpxchg loop with old in rax, the candidate in rdx and
        // the operand read in place (an imm32, its register home, or a
        // stack-staged copy). 128-bit atomics stage arguments in
        // rdi/rsi/rdx/rcx (and rbx for cmpxchg16b).
        Instruction::AtomicRmw { op, ty, .. } => {
            if matches!(ty, IrType::I128 | IrType::U128) {
                RDX_RDI
            } else {
                match op {
                    AtomicRmwOp::Add
                    | AtomicRmwOp::Sub
                    | AtomicRmwOp::Xchg
                    | AtomicRmwOp::TestAndSet => NONE,
                    AtomicRmwOp::And | AtomicRmwOp::Or | AtomicRmwOp::Xor | AtomicRmwOp::Nand => {
                        RDX
                    }
                }
            }
        }
        // cmpxchg: desired in rdx (128-bit cmpxchg16b also clobbers rbx, and
        // libatomic fallback stages in rdi/rsi/rdx/rcx/r8). Atomic store:
        // value in rdx (128-bit store stages in rdi/rsi/rdx/rcx).
        Instruction::AtomicCmpxchg { ty, .. } => {
            if matches!(ty, IrType::I128 | IrType::U128) {
                RDX_RDI
            } else {
                RDX
            }
        }
        Instruction::AtomicStore { ty, .. } => {
            if matches!(ty, IrType::I128 | IrType::U128) {
                RDX_RDI
            } else {
                RDX
            }
        }
        Instruction::AtomicLoad { ty, .. } => {
            if matches!(ty, IrType::I128 | IrType::U128) {
                RDX_RDI
            } else {
                NONE
            }
        }
        Instruction::Intrinsic { op, .. } => match op {
            IntrinsicOp::Rdtscp => RDX_RDI,
            // GCC __builtin_apply family and __builtin_longjmp: the emitters
            // read/write the raw %rdx (DoBuiltinApply stages arg3 into it AND
            // performs a real `call *%r11` that the IR does not model as a
            // call point; RestoreApplyResult/BuiltinLongjmp reload it from
            // memory). None of these may coexist with a %rdx home.
            IntrinsicOp::RestoreApplyResult
            | IntrinsicOp::DoBuiltinApply
            | IntrinsicOp::BuiltinLongjmp => RDX,
            IntrinsicOp::Rdtsc
            | IntrinsicOp::F128Copysign
            | IntrinsicOp::FmaF64x2
            | IntrinsicOp::FmaF64x4
            | IntrinsicOp::FmaF64x4Hoisted
            | IntrinsicOp::FmaF64x4SIB
            | IntrinsicOp::FmaF64x4HoistedSIB
            | IntrinsicOp::MulAddF64x4HoistedSIB
            | IntrinsicOp::LoadF64x4
            | IntrinsicOp::LoadF64x2
            | IntrinsicOp::LoadI32x8
            | IntrinsicOp::LoadI32x4
            | IntrinsicOp::VecZeroI32x8
            | IntrinsicOp::VecZeroI32x4 => RDX,
            _ => NONE,
        },
        _ => NONE,
    }
}

/// Union of [`x86_inst_fixed_scratch`] over the whole body, plus the
/// jump-table dispatch of `Switch` terminators (`%rdx`).
pub fn x86_body_fixed_scratch(func: &IrFunction) -> X86FixedScratch {
    let mut acc = X86FixedScratch::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            acc = acc.or(x86_inst_fixed_scratch(inst));
            if acc.rdx && acc.rdi {
                return acc;
            }
        }
        if matches!(block.terminator, Terminator::Switch { .. }) {
            acc.rdx = true;
        }
    }
    acc
}

/// x86-64 program points whose emitter provably clobbers `%rdx`
/// (PhysReg 16), in the flat liveness numbering — one point per instruction
/// plus one per terminator, the same walk `LivenessResult` and
/// `collect_i686_scratch_hazard_points` use, so `LiveInterval::start/end`
/// compare directly against these points.
///
/// The classification is DERIVED from [`x86_inst_fixed_scratch`] (plus the
/// `Switch`-terminator jump-table dispatch that
/// [`x86_body_fixed_scratch`] adds) rather than re-spelled, so the
/// allocator's view of `%rdx` clobbers can never drift from the pool gate's
/// view — the exact "emitter and allocator disagree" defect class this
/// repository has been burned by before (i686 divrem pairs derive from the
/// same IR for the same reason).
///
/// Completeness argument (why hazard points are the ONLY `%rdx` clobbers):
/// `%rdx` already sits in the caller-saved pool for functions with NO
/// clobber-class instruction (prologue.rs admission gate), so every
/// emitter outside the clobber classes is already exercised with
/// `%rdx`-homed values in production and provably leaves `%rdx` intact
/// (the register-direct fast paths are home-generic;
/// `emit_save_acc_impl` switches to `%r11` when any value is `%rdx`-homed;
/// `const_offset_fold_reg_base_ok` refuses `%rdx`/`%r11` bases; the divrem
/// pair fusion carries an explicit `%rdx`-home screening). Call-shaped
/// clobbers (calls, inline asm with register clobbers, memcpy, i128
/// div/rem helper calls) are handled by `call_points`: a value live across
/// them is never a Phase-2 (caller-saved) candidate at all.
pub fn collect_x64_rdx_clobber_points(func: &IrFunction) -> Vec<u32> {
    let mut points: Vec<u32> = Vec::new();
    let mut point: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if x86_inst_fixed_scratch(inst).rdx {
                points.push(point);
            }
            point += 1;
        }
        // Switch dispatch: `leaq .LJTI(%rip),%rdx` (or the equivalent table
        // walk) at the terminator point.
        if matches!(block.terminator, Terminator::Switch { .. }) {
            points.push(point);
        }
        point += 1;
    }
    points
}

/// Whether the body touches any I128/U128 value — the conservative gate for
/// the position-aware `%rdx` admission wave below.
///
/// This is VALUE-based, not instruction-shape-based: an i128 value that flows
/// through `Copy`/`Phi` (phi-elimination materialises exactly such Copies,
/// and `emit_copy_i128_impl` unconditionally writes the `%rax:%rdx` pair)
/// must keep the whole body on the historical whole-function `%rdx`
/// exclusion — a typed-field scan would miss the Copy points and let the
/// wave home a value live across one. Detection is therefore a small
/// fixpoint: values with a typed wide def (BinOp/UnaryOp/Cast/Load/
/// ParamRef/Select/AtomicLoad/AtomicRmw/AtomicCmpxchg/Phi/Call result),
/// wide-typed call-argument values, `IrConst::I128` constants, and every
/// `Copy` destination chained from an already-wide source. Wide bodies were
/// never production-exercised with `%rdx`-homed values (the pool gate always
/// excluded `%rdx` there), so point-local emitter completeness is asserted,
/// not proven, for that class; every other emitter is exercised with
/// `%rdx` homes by the clobber-free bodies that admit `%rdx` today.
pub fn x86_body_has_wide_ops(func: &IrFunction) -> bool {
    let wide = |ty: &IrType| matches!(ty, IrType::I128 | IrType::U128);
    if wide(&func.return_type) {
        return true;
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            let typed_wide = match inst {
                Instruction::BinOp { ty, .. }
                | Instruction::UnaryOp { ty, .. }
                | Instruction::Cmp { ty, .. }
                | Instruction::Load { ty, .. }
                | Instruction::Store { ty, .. }
                | Instruction::ParamRef { ty, .. }
                | Instruction::Select { ty, .. }
                | Instruction::AtomicLoad { ty, .. }
                | Instruction::AtomicStore { ty, .. }
                | Instruction::AtomicRmw { ty, .. }
                | Instruction::AtomicCmpxchg { ty, .. }
                | Instruction::AtomicInc { ty, .. }
                | Instruction::Phi { ty, .. } => wide(ty),
                Instruction::Cast { from_ty, to_ty, .. } => wide(from_ty) || wide(to_ty),
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    wide(&info.return_type) || info.arg_types.iter().any(wide)
                }
                _ => false,
            };
            if typed_wide {
                return true;
            }
            let mut wide_constant = false;
            for_each_operand_in_instruction(inst, |operand| {
                if matches!(operand, Operand::Const(IrConst::I128(_))) {
                    wide_constant = true;
                }
            });
            if wide_constant {
                return true;
            }
        }
        let mut wide_constant = false;
        for_each_operand_in_terminator(&block.terminator, |operand| {
            if matches!(operand, Operand::Const(IrConst::I128(_))) {
                wide_constant = true;
            }
        });
        if wide_constant {
            return true;
        }
    }
    false
}

/// Whether the x86-64 body contains an instruction whose 128-bit emitter
/// uses `%r8`/`%r9`/`%rdi`/`%rsi` as scratch (`i128_ops.rs`: the binop RHS
/// high half in `%rsi`, `mul` in `%r8`/`%r9`/`%rdi`/`%rsi`, compares in
/// `%r8b`, the divrem/float helper staging in `%rdi`/`%rsi`, and the wide
/// atomics' libatomic staging). The prologue withdraws those four registers
/// from the general caller-saved pool for such bodies; the shared predicate
/// keeps the pool gate and the allocator's hazard wave
/// ([`collect_x64_wide_hazard_points`]) from ever disagreeing about which
/// bodies are "wide".
pub fn x86_body_has_i128_ops(func: &IrFunction) -> bool {
    let wide = |ty: &IrType| matches!(ty, IrType::I128 | IrType::U128);
    func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| match inst {
            Instruction::BinOp { ty, .. }
            | Instruction::UnaryOp { ty, .. }
            | Instruction::Cmp { ty, .. }
            | Instruction::Store { ty, .. }
            | Instruction::AtomicLoad { ty, .. }
            | Instruction::AtomicStore { ty, .. }
            | Instruction::AtomicRmw { ty, .. }
            | Instruction::AtomicCmpxchg { ty, .. } => wide(ty),
            Instruction::Cast { from_ty, to_ty, .. } => wide(from_ty) || wide(to_ty),
            _ => false,
        })
    })
}

/// x86-64 program points (flat liveness numbering: one per instruction plus
/// one per terminator, as in [`collect_x64_rdx_clobber_points`]) at which a
/// 128-bit emitter may write `%r8`/`%r9`/`%rdi`/`%rsi`.
///
/// Bodies with [`x86_body_has_i128_ops`] lose those registers for the whole
/// function, yet the clobbers are point-local: every wide emitter stages
/// through fixed scratch within its own instruction and keeps no state in
/// them across points (the only cross-point wide state is the `%rax:%rdx`
/// accumulator pair, which this wave never hands out). A value whose live
/// range avoids every point below is exactly as safe in those registers as
/// in a narrow body.  The set is deliberately a superset of the real
/// clobbers:
///
/// * every instruction that is wide-typed, defines or reads a wide VALUE
///   (fixpoint through `Copy`, since phi elimination materialises wide
///   copies that `emit_copy_i128_impl` stages), or carries an `I128`
///   constant, and every terminator that reads one;
/// * the FUSION WINDOW of every wide `Cmp`: the backend may emit a compare
///   at a later consumer in the same block (`detect_cmp_branch_fusion`,
///   fused selects), so every point from the `Cmp` to its block's
///   terminator counts.
///
/// The one exception is an integer `Cast` into or out of a wide type: its
/// emitter stages through `%rax:%rdx` only (the carrier unpack
/// `trunc(v >> 64)` and the pack's `zext` were the commonest "hazards" in
/// practice, and each one split every wave candidate's range around it).
///
/// Callers must apply the hazards inclusively at both ends
/// (`overlaps_inclusive`, no birth exemption): a wide emitter's dest store is not guaranteed to follow its
/// last scratch use for every shape.
pub fn collect_x64_wide_hazard_points(func: &IrFunction) -> Vec<u32> {
    let wide = |ty: &IrType| matches!(ty, IrType::I128 | IrType::U128);
    let typed_wide = |inst: &Instruction| match inst {
        Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Cmp { ty, .. }
        | Instruction::Load { ty, .. }
        | Instruction::Store { ty, .. }
        | Instruction::ParamRef { ty, .. }
        | Instruction::Select { ty, .. }
        | Instruction::AtomicLoad { ty, .. }
        | Instruction::AtomicStore { ty, .. }
        | Instruction::AtomicRmw { ty, .. }
        | Instruction::AtomicCmpxchg { ty, .. }
        | Instruction::Phi { ty, .. } => wide(ty),
        Instruction::Cast { from_ty, to_ty, .. } => wide(from_ty) || wide(to_ty),
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            wide(&info.return_type) || info.arg_types.iter().any(wide)
        }
        _ => false,
    };
    // Wide values: typed wide defs, then Copy chains to a fixpoint.
    let mut wide_vals: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let defines_wide = match inst {
                Instruction::Cast { to_ty, .. } => wide(to_ty),
                Instruction::Cmp { .. } | Instruction::Store { .. } => false,
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    wide(&info.return_type)
                }
                _ => typed_wide(inst),
            };
            if defines_wide {
                if let Some(d) = inst.dest() {
                    wide_vals.insert(d.0);
                }
            }
        }
    }
    loop {
        let before = wide_vals.len();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } = inst
                {
                    if wide_vals.contains(&v.0) {
                        wide_vals.insert(dest.0);
                    }
                }
            }
        }
        if wide_vals.len() == before {
            break;
        }
    }
    let has_wide_const = |inst: &Instruction| {
        let mut found = false;
        for_each_operand_in_instruction(inst, |operand| {
            found |= matches!(operand, Operand::Const(IrConst::I128(_)));
        });
        found
    };
    let mut points: Vec<u32> = Vec::new();
    let mut point: u32 = 0;
    for block in &func.blocks {
        let mut window_open = false;
        for inst in &block.instructions {
            if matches!(inst, Instruction::Cmp { ty, .. } if wide(ty)) {
                window_open = true;
            }
            // Integer casts into or out of a wide value stage only through
            // the `%rax:%rdx` pair (`emit_cast_default`: `operand_to_rax_rdx`
            // / `store_rax_rdx_to`, a narrow `emit_store_result`); the
            // float<->I128 casts are libcalls and already call points
            // (`liveness::instruction_is_call_point`), which no Phase-2
            // candidate spans.
            let int_cast = matches!(inst, Instruction::Cast { from_ty, to_ty, .. }
                if from_ty.is_integer() && to_ty.is_integer());
            let mut touches = !int_cast && (typed_wide(inst) || has_wide_const(inst));
            if !touches && !int_cast {
                inst.for_each_used_value(|v| touches |= wide_vals.contains(&v));
                touches |= inst.dest().is_some_and(|d| wide_vals.contains(&d.0));
            }
            if touches || window_open {
                points.push(point);
            }
            point += 1;
        }
        let mut term_touches = window_open;
        block
            .terminator
            .for_each_used_value(|v| term_touches |= wide_vals.contains(&v));
        for_each_operand_in_terminator(&block.terminator, |operand| {
            term_touches |= matches!(operand, Operand::Const(IrConst::I128(_)));
        });
        if term_touches {
            points.push(point);
        }
        point += 1;
    }
    points
}

/// Companion of [`x86_param_caller_homes_safe`]: with parameters parked in
/// their incoming registers for the whole body, an inline-expanded
/// `memcpy`/`memset` (which clobbers %rdi/%rsi/%rcx/%rax/%xmm0/%xmm1) is
/// harmless exactly when no parameter value is read after it.  Only
/// expansions in the entry block are admitted; an expansion in any other
/// block (loop bodies, conditional paths) keeps today's conservative policy
/// because "after" would need dominance and back-edge reasoning.
fn x86_params_dead_after_inline_libc_calls(
    func: &IrFunction,
    is_inline: &dyn Fn(&Instruction) -> bool,
) -> bool {
    let Some(entry) = func.blocks.first() else {
        return true;
    };
    let params: FxHashSet<u32> = entry
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            Instruction::ParamRef { dest, .. } => Some(dest.0),
            _ => None,
        })
        .collect();
    let uses_param = |inst: &Instruction| {
        let mut used = false;
        inst.for_each_used_value(|v| used |= params.contains(&v));
        used
    };
    let mut seen_inline = false;
    for inst in &entry.instructions {
        if seen_inline && uses_param(inst) {
            return false;
        }
        if is_inline(inst) {
            seen_inline = true;
        }
    }
    if seen_inline {
        let mut used = false;
        entry
            .terminator
            .for_each_used_value(|v| used |= params.contains(&v));
        if used {
            return false;
        }
    }
    for block in func.blocks.iter().skip(1) {
        if block.instructions.iter().any(|inst| is_inline(inst)) {
            return false;
        }
        if seen_inline {
            if block.instructions.iter().any(|inst| uses_param(inst)) {
                return false;
            }
            let mut used = false;
            block
                .terminator
                .for_each_used_value(|v| used |= params.contains(&v));
            if used {
                return false;
            }
        }
    }
    true
}

/// Whether a body's `%rdx` clobbers still let x86-64 parameters take
/// caller-saved homes ([`x86_param_caller_homes_safe_with_config`]).
///
/// The `idivq` defect that gate records was a parameter HOMED in `%rdx`
/// across the clobber. That hazard is now owned where it arises: the prologue
/// drops `%rdx` from the pool of any clobbering body
/// ([`x86_body_fixed_scratch`]), hints are honoured only inside the pool
/// (`LinearScanAllocator::find_free_register`), and the Phase-2x64 wave
/// admits `%rdx` only for intervals that miss every clobber point
/// ([`collect_x64_rdx_clobber_points`]). A parameter WITHOUT a home is read
/// from its ABI register at its `ParamRef`, which the gate already confines
/// to the entry prefix -- before the first clobber can execute.  Keeping
/// every parameter out of the caller-saved pool for the whole function was
/// therefore pure cost: `x / y` pushed %rbx and %r12 and copied both
/// arguments into them (GCC: `movq %rdi,%rax; xorl %edx,%edx; divq %rsi`).
///
/// The admitted clobber classes are those whose only `%rdx` write is the
/// instruction itself, at its own program point: non-wide division,
/// remainder and multiply-high, non-wide atomics (cmpxchg, atomic store,
/// the bitwise RMW loop), and `Switch` jump-table dispatch.  Excluded:
/// wide (I128/U128) bodies -- `emit_copy_i128_impl` and the wide ParamRef
/// and store paths stage through `%rax:%rdx`, which the entry prefix itself
/// may execute ahead of a later parameter's ABI read (the 128-bit atomics
/// also stage arguments in %rdi/%rsi); F128 stores; and the
/// `__builtin_apply`/`__builtin_longjmp` intrinsics, whose emitters perform
/// an unmodelled `call *%r11`.  Without the wave (`CCC_NO_RDX_HAZARD`) the
/// relaxation buys nothing, so that switch restores the old gate too.
fn x86_rdx_clobbers_allow_param_homes(func: &IrFunction, ra_config: &RaConfig) -> bool {
    if ra_config.no_rdx_param_homes || ra_config.no_rdx_hazard {
        return false;
    }
    let narrow_divlike = |inst: &Instruction| {
        matches!(
            inst,
            Instruction::BinOp { op, ty, .. }
                if !matches!(ty, IrType::I128 | IrType::U128)
                    && matches!(
                        op,
                        IrBinOp::SDiv
                            | IrBinOp::UDiv
                            | IrBinOp::SRem
                            | IrBinOp::URem
                            | IrBinOp::UMulHigh
                            | IrBinOp::SMulHigh
                    )
        )
    };
    // Narrow atomics write %rdx only inside their own sequence (cmpxchg's
    // `desired`, an atomic store's value, the RMW loop's candidate), after
    // which none of their operands is read again -- and the wave refuses any
    // value live INTO the point, so no operand is %rdx-homed.  128-bit forms
    // are wide and excluded below.
    let narrow_atomic = |inst: &Instruction| {
        matches!(
            inst,
            Instruction::AtomicRmw { ty, .. }
                | Instruction::AtomicCmpxchg { ty, .. }
                | Instruction::AtomicStore { ty, .. }
                if !matches!(ty, IrType::I128 | IrType::U128)
        ) && !x86_inst_fixed_scratch(inst).rdi
    };
    let all_admitted = func.blocks.iter().all(|block| {
        block.instructions.iter().all(|inst| {
            !x86_inst_fixed_scratch(inst).rdx || narrow_divlike(inst) || narrow_atomic(inst)
        })
    });
    all_admitted && !x86_body_has_wide_ops(func)
}

/// Compatibility query for isolated callers. Production code must use the
/// explicit invocation policy variant below.
pub fn x86_param_caller_homes_safe(func: &IrFunction) -> bool {
    x86_param_caller_homes_safe_with_config(func, &RaConfig::default())
}

pub(crate) fn x86_param_caller_homes_safe_with_config(
    func: &IrFunction,
    ra_config: &RaConfig,
) -> bool {
    if func.blocks.is_empty() || ra_config.no_leaf_param_gpr {
        return false;
    }
    // Multi-block expansion is limited to the six SysV register arguments.
    // Stack arguments need a stable entry-RSP frame model; moving those homes
    // changes their offsets (the nine-argument regression catches this).
    if func.blocks.len() > 1 && func.params.len() > 6 {
        return false;
    }
    // Fixed-size `memcpy` / `memset` calls that the x86-64 backend expands
    // inline are not calls: the expansion touches only %rdi/%rsi/%rcx/%rax/
    // %xmm0/%xmm1.  They still count as call points for every other value
    // (liveness), but they must not evict the *parameters* from their ABI
    // registers — that rule alone made `void z(char *p){memset(p,0,15);}`
    // 8 instructions (push/sub/mov/add/pop around two stores) where GCC,
    // Clang and ICX emit 3.  The exemption is sound only when no parameter
    // is read after the expansion (`x86_params_dead_after_inline_libc_calls`).
    let is_64 = !crate::common::types::target_is_32bit();
    let inline_libc = |inst: &Instruction| -> bool {
        is_64
            && matches!(inst, Instruction::Call { func: name, info }
                if crate::backend::generation::inline_memcpy_len(name, info).is_some()
                    || crate::backend::generation::x86_inline_memset_len(name, info).is_some())
    };
    // Real calls do not disqualify: every consumer of this predicate grants
    // a parameter a caller-saved home only through the allocator's Phase 2,
    // which refuses call-spanning values and keeps later call arguments out
    // of the argument registers (`RegAllocConfig::call_arg_regs`) -- the same
    // guarantees every other caller-saved home in a non-leaf function rests
    // on. So a parameter read only before the first call stays in its ABI
    // register (`f(p) { x = p->a; g(x); }` paid push/mov/pop of %rbx for it),
    // and one live across a call still takes a callee-saved home. Inline asm
    // may clobber argument registers without a precise clobber list: it
    // keeps the conservative path.
    if func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::InlineAsm { .. }))
    }) {
        return false;
    }
    if !x86_params_dead_after_inline_libc_calls(func, &inline_libc) {
        return false;
    }
    // The pre-store model parks each register param in its incoming ABI
    // register for the WHOLE body. Any instruction that implicitly clobbers
    // %rdx (division via cqto/cltd, i128 rax:rdx pairs, switch jump tables,
    // fixed-scratch intrinsics) destroys a param homed there behind the
    // allocator's back. Reproduced: `(uint8_t)p2` read from %dl after an
    // `idivq`'s cqto zeroed it — every truncation of the param silently
    // wrong (stress intexpr seed 1, O1 rt+cf, got 65533 expected 65532).
    // Mirror the prologue's allocation exclusions (see
    // x86_body_fixed_scratch): when the body can clobber %rdi, or %rdx in a
    // way the position-aware model does not cover
    // ([`x86_rdx_clobbers_allow_param_homes`]), params fall back to the
    // ordinary spill-home path.  The %rdi half matters for
    // future-proofing: today every rdi clobberer (rdtscp, the 128-bit
    // atomics) is an RDX_RDI pair, so the historical rdx-only gate happened
    // to catch them transitively — but the prologue already removes %rdi
    // from the pool independently, and a future rdi-only scratch would
    // otherwise silently destroy a param parked in %rdi.
    let caller_home_scratch = x86_body_fixed_scratch(func);
    if caller_home_scratch.rdi
        || (caller_home_scratch.rdx && !x86_rdx_clobbers_allow_param_homes(func, ra_config))
    {
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
    /// x86-64 IPA-RA: caller-saved registers (also in `caller_saved_regs`)
    /// that survive every call of this function; registers no phase used
    /// receive slotted call-spanning values or a callee-saved register's
    /// holders after allocation (`ipa_fill_preserved_regs`). Empty elsewhere.
    pub ipa_regs: Vec<PhysReg>,
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
    /// Backend-folded operand reads: map of `operand value id` → consumer
    /// value ids whose emission re-reads the operand's register with no IR
    /// operand recording it — the (peeled) index of a SIB/indexed-folded GEP,
    /// read at the consumer's Load/Store, and CMP-REPLAY operands, re-compared
    /// at the Cmp's consumers. The allocator hands the map to
    /// `compute_live_intervals_with_hidden_reads`, which treats the operand as
    /// used at each consumer's definition and uses inside the liveness
    /// dataflow; otherwise the operand's register is free for reuse and the
    /// emitted `[base, index, lsl #N]` reads whatever moved in (reproduced:
    /// fa[i] stores landing on fa[seed] on aarch64 -O0/-O2; sqlite
    /// `wherePathSolver` indexing `aLoop[]` with a pointer on x86-64).
    pub folded_index_uses: FxHashMap<u32, Vec<u32>>,
    /// Registers in which the backend accepts a constant-offset fold base:
    /// a `GEP base, C` / `base ± C` whose every use is a Load/Store address
    /// is then emitted as `C(%base)` at each access and never materialised
    /// (`const_offset_fold_reg_base_ok`; x86-64 and i686). Such an address
    /// needs no register of its own once its base holds one of these, so the
    /// allocator DEFERS it (see `defer_const_fold_addresses`). Empty: off.
    pub fold_base_regs: Vec<PhysReg>,
    /// Fold addresses whose deferral already failed in an earlier round (the
    /// base ended without a register in `fold_base_regs`): they stay ordinary
    /// candidates. Filled by `run_regalloc_and_merge_clobbers_ex`.
    pub fold_readmit: FxHashSet<u32>,
    /// ABI-preferred homes (e.g. an incoming ParamRef already in `%rdi`).
    /// Hints never override `follow_value` and are honored only when the
    /// physical register belongs to the current allocation wave.
    pub reg_hints: FxHashMap<u32, PhysReg>,
    /// Invocation-scoped policy captured by the driver before code generation.
    pub ra_config: Arc<RaConfig>,
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

/// Conservative envelope for every value.
///
/// Production liveness emits one fat interval per defined value. If a caller
/// still supplies several, keep the complete envelope rather than last-write
/// coverage that silently drops an earlier range.
fn interval_map(liveness: &LivenessResult) -> FxHashMap<u32, (u32, u32)> {
    let mut m: FxHashMap<u32, (u32, u32)> = FxHashMap::default();
    m.reserve(liveness.intervals.len());
    for iv in &liveness.intervals {
        debug_assert!(
            iv.start <= iv.end,
            "invalid live interval for v{}: [{}, {}]",
            iv.value_id,
            iv.start,
            iv.end
        );
        if let Some(bounds) = m.get_mut(&iv.value_id) {
            bounds.0 = bounds.0.min(iv.start);
            bounds.1 = bounds.1.max(iv.end);
        } else {
            m.insert(iv.value_id, (iv.start, iv.end));
        }
    }
    m
}

fn intervals_overlap(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

fn summed_use_weight(values: &[u32], use_count: &FxHashMap<u32, u64>) -> u64 {
    values.iter().fold(0u64, |total, value| {
        total.saturating_add(use_count.get(value).copied().unwrap_or(0))
    })
}

/// Resolve a root in an acyclic union-find forest.
///
/// Missing entries are singleton roots. The forest must be constructed
/// through root-to-root unions; arbitrary parent cycles are invalid.
fn allocation_class_root(parent: &FxHashMap<u32, u32>, value: u32) -> u32 {
    let mut root = value;
    while let Some(&next) = parent.get(&root) {
        if next == root {
            break;
        }
        root = next;
    }
    root
}

/// Normalize all entries before direct parent lookups are used as class ids.
fn flatten_allocation_classes(parent: &mut FxHashMap<u32, u32>) {
    let values: Vec<u32> = parent.keys().copied().collect();
    for value in values {
        let root = allocation_class_root(parent, value);
        parent.insert(value, root);
    }
}

/// Enumerate conflicts using the existing allocator boundary predicate.
///
/// Input: `(physical register, start, end, allocation class)`.
/// Output: `(physical register, lower class id, higher class id,
/// overlap start, overlap end)`.
///
/// Preserves `intervals_overlap`, including its treatment of zero-length
/// ranges. Production callers normalize coverage within each
/// `(register, class)` before calling this function.
fn overlapping_class_spans(mut spans: Vec<(u8, u32, u32, u32)>) -> Vec<(u8, u32, u32, u32, u32)> {
    spans.sort_unstable();
    let mut out = Vec::new();
    let mut active: Vec<(u32, u32, u32)> = Vec::new();
    let mut current_reg: Option<u8> = None;
    for (reg, start, end, class) in spans {
        assert!(
            start <= end,
            "invalid register-allocation range: r{} class v{} [{}, {}]",
            reg,
            class,
            start,
            end
        );
        if current_reg != Some(reg) {
            active.clear();
            current_reg = Some(reg);
        }
        active.retain(|&(_, active_end, _)| active_end > start);
        for &(other_start, other_end, other_class) in &active {
            if other_class != class && intervals_overlap((other_start, other_end), (start, end)) {
                out.push((
                    reg,
                    other_class.min(class),
                    other_class.max(class),
                    other_start.max(start),
                    other_end.min(end),
                ));
            }
        }
        // A zero-length range can conflict with an earlier enclosing range
        // under `intervals_overlap`, but cannot conflict with a subsequent
        // range whose start is >= its end.
        if start < end {
            active.push((start, end, class));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Shared homes must satisfy the union of member restrictions.
/// `member_of` maps directly to final owners.
fn propagate_member_restrictions(restricted: &mut FxHashSet<u32>, member_of: &FxHashMap<u32, u32>) {
    let owners: Vec<u32> = member_of
        .iter()
        .filter_map(|(&member, &owner)| restricted.contains(&member).then_some(owner))
        .collect();
    restricted.extend(owners);
}

fn allocation_owner_bounds(
    value: u32,
    merged_of: &FxHashMap<u32, LiveInterval>,
    iv_map: &FxHashMap<u32, (u32, u32)>,
) -> Option<(u32, u32)> {
    merged_of
        .get(&value)
        .map(|interval| (interval.start, interval.end))
        .or_else(|| iv_map.get(&value).copied())
}

/// Per-owner coverage, including a fat-interval fallback for every value
/// that has no segment data.
///
/// A single segmented member must not hide an unsegmented member of the
/// same allocation web.
fn owned_live_segments(
    liveness: &LivenessResult,
    member_of: &FxHashMap<u32, u32>,
) -> FxHashMap<u32, Vec<(u32, u32)>> {
    let owner_of = |value: u32| member_of.get(&value).copied().unwrap_or(value);
    let mut owned: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for segment in &liveness.segments {
        segmented.insert(segment.value_id);
        owned
            .entry(owner_of(segment.value_id))
            .or_default()
            .push((segment.start, segment.end));
    }
    for interval in &liveness.intervals {
        if !segmented.contains(&interval.value_id) {
            owned
                .entry(owner_of(interval.value_id))
                .or_default()
                .push((interval.start, interval.end));
        }
    }
    for pieces in owned.values_mut() {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_segment_union(pieces, &source);
    }
    owned
}

/// Linear merge-style interference test for sorted hole-aware segment sets.
/// Uses the allocator's half-open boundary convention: one value dying at the
/// exact point another is born may hand the register directly to it.
fn guard_internal_hole_boundaries(segments: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let last = segments.len().saturating_sub(1);
    segments
        .iter()
        .enumerate()
        .map(|(i, &(start, end))| {
            (
                if i > 0 {
                    start.saturating_sub(1)
                } else {
                    start
                },
                if i < last { end.saturating_add(1) } else { end },
            )
        })
        .collect()
}

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

/// Phase 2f admission step: place one segment candidate into the first pooled
/// register whose occupancy it does not interfere with, and record the
/// placement. The interference test and the occupancy update both use the
/// internal-hole-guarded form of the candidate's segments — the same guarded
/// form the pre-existing holders were loaded with. Keeping the test and the
/// insertion in one function makes the guarded spelling the only one: an
/// earlier open-coded version tested the guarded segments but inserted the
/// raw ones, so a later candidate could be admitted into the exact internal
/// hole of an already-placed co-holder (touching both hole boundaries, which
/// the guard exists to forbid) — admission depended on placement order,
/// which a register interference relation must never do.
fn try_fill_segment_candidate(
    candidate_segments: &[(u32, u32)],
    occupied_by_reg: &mut FxHashMap<u8, Vec<(u32, u32)>>,
    pool: &[PhysReg],
) -> Option<PhysReg> {
    let guarded_candidate = guard_internal_hole_boundaries(candidate_segments);
    let reg = pool.iter().copied().find(|reg| {
        occupied_by_reg
            .get(&reg.0)
            .is_none_or(|occupied| !segment_sets_overlap(&guarded_candidate, occupied))
    })?;
    insert_segment_union(
        occupied_by_reg.entry(reg.0).or_default(),
        &guarded_candidate,
    );
    Some(reg)
}

/// Phase 2c tier 2 (RA-PHASE2C-SAVED-REUSE): a no-eviction fill of the free
/// stretches of already-saved callee-saved registers. `candidates` are
/// `(loop-weighted web pressure, envelope length, value, half-open span)`;
/// they are placed hottest first, then shorter envelope, then value id (Phase
/// 2f's ranking), each into the first register in `regs` order whose
/// normalized occupancy union in `occupied` it does not overlap. `occupied`
/// is updated in place. Returns the placements in placement order.
fn fill_saved_callee_stretches(
    mut candidates: Vec<(u64, u32, u32, (u32, u32))>,
    occupied: &mut FxHashMap<u8, Vec<(u32, u32)>>,
    regs: &[PhysReg],
) -> Vec<(u32, PhysReg)> {
    candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut placed = Vec::new();
    for (_, _, value, span) in candidates {
        let piece = [span];
        let Some(reg) = regs.iter().copied().find(|reg| {
            occupied
                .get(&reg.0)
                .is_none_or(|spans| !segment_sets_overlap(&piece, spans))
        }) else {
            continue;
        };
        insert_segment_union(occupied.entry(reg.0).or_default(), &piece);
        placed.push((value, reg));
    }
    placed
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

/// Sorted, coalesced coverage pieces of two values overlap.
///
/// Used by general coalescing so a phi-web exemption is hole-aware: exclusive
/// CFG arms may share a home, simultaneously-live segments may not.
fn sorted_coverage_overlaps(left: &[(u32, u32)], right: &[(u32, u32)]) -> bool {
    let mut i = 0usize;
    let mut j = 0usize;
    while i < left.len() && j < right.len() {
        if intervals_overlap(left[i], right[j]) {
            return true;
        }
        if left[i].1 <= right[j].0 {
            i += 1;
        } else {
            j += 1;
        }
    }
    false
}

fn coverage_of_value(
    value: u32,
    segments_of: &FxHashMap<u32, Vec<(u32, u32)>>,
    iv_map: &FxHashMap<u32, (u32, u32)>,
) -> Vec<(u32, u32)> {
    if let Some(pieces) = segments_of.get(&value) {
        return pieces.clone();
    }
    iv_map
        .get(&value)
        .copied()
        .map(|span| vec![span])
        .unwrap_or_default()
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
    let owned = owned_live_segments(liveness, coalesce_member_of);
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
        // Hole-aware scanning exists to model real gaps. A single contiguous
        // segment adds no hole information and can shrink the historical fat
        // interval based on local block touches; keep envelope semantics there.
        if inside && segs.len() > 1 {
            range.set_segments(segs.clone());
        }
    }
}

/// Whether `config` is the i686 `{%ecx, %edx}` caller-saved pool that the
/// i686-only phases (Phase 2's scratch-hazard split, 2d, 2e, 2g, 2h) model.
/// PhysReg 4/5 are `%ecx`/`%edx` only on i686: AArch64 numbers x4/x5 the
/// same, and keying on the pool alone ran AArch64 through the i686 hazard
/// model (so its Phase 2 only ever offered x4/x5, and Phase 2e's "%eax"
/// home was x6) from 335fcf88c until this target check.
fn i686_scratch_pool(config: &RegAllocConfig) -> bool {
    const EM_386: u16 = 3;
    crate::common::types::target_elf_machine() == EM_386
        && config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 4 | 5))
}

/// Live *across* a clobber: defined before it, used after it.
/// Born at a call (retval) or dying at a call (arg) may use caller-saved.
#[inline]
fn spans_any_call(iv: &LiveInterval, call_points: &[u32]) -> bool {
    let idx = call_points.partition_point(|&cp| cp <= iv.start);
    idx < call_points.len() && call_points[idx] < iv.end
}

/// Half-open `[start, end)` occupancy against a sorted point list.
///
/// Unlike `spans_any_call`, a point exactly at `start` counts: synthetic
/// slot contents can already be live at a block boundary.
fn half_open_range_contains_any_point(start: u32, end: u32, points: &[u32]) -> bool {
    if start >= end {
        return false;
    }
    let index = points.partition_point(|&point| point < start);
    index < points.len() && points[index] < end
}

/// Owners whose live coverage includes a call after their definition.
///
/// Segment data is authoritative when present. Values with no segments fall
/// back to the fat envelope so a missing-segment owner is never treated as
/// "not call-spanning".
fn collect_call_spanning_owners(
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    coalesce_member_of: &FxHashMap<u32, u32>,
    call_points: &[u32],
) -> FxHashSet<u32> {
    let owner_of = |value: u32| coalesce_member_of.get(&value).copied().unwrap_or(value);
    let mut set = FxHashSet::default();
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for seg in &liveness.segments {
        segmented.insert(seg.value_id);
        let def = iv_map
            .get(&seg.value_id)
            .map(|&(s, _)| s)
            .unwrap_or(seg.start);
        let idx = call_points.partition_point(|&cp| cp < seg.start || cp <= def);
        if idx < call_points.len() && call_points[idx] < seg.end {
            set.insert(owner_of(seg.value_id));
        }
    }
    for interval in &liveness.intervals {
        if !segmented.contains(&interval.value_id) && spans_any_call(interval, call_points) {
            set.insert(owner_of(interval.value_id));
        }
    }
    set
}

/// Call points a segment is live ACROSS: defined before the call and live
/// after it (`def < cp`, `seg.start <= cp < seg.end`) -- the predicate of
/// [`collect_call_spanning_owners`].
fn segment_spanned_calls<'a>(
    seg: &LiveInterval,
    def: u32,
    call_points: &'a [u32],
) -> impl Iterator<Item = u32> + 'a {
    let lo = call_points.partition_point(|&cp| cp < seg.start || cp <= def);
    let (start, end) = (seg.start, seg.end);
    call_points[lo..]
        .iter()
        .copied()
        .take_while(move |&cp| cp < end && cp >= start)
}

/// Inputs of [`ipa_fill_preserved_regs`] beyond the assignment map.
struct IpaFill<'a> {
    /// Caller-saved registers every call of the function preserves (x86-64
    /// `ipa_ra`): a home there spans calls with no save/restore anywhere.
    ipa_regs: &'a [PhysReg],
    /// Callee-saved registers whose holders may move wholesale to an IPA
    /// register (renaming), scanned last-first.
    callee_pool: &'a [PhysReg],
    /// Caller-saved registers a displaced tenant of an IPA register may move to.
    reloc_pool: &'a [PhysReg],
    iv_map: &'a FxHashMap<u32, (u32, u32)>,
    /// Loop-weighted use count: the spill traffic a home saves.
    cost: &'a dyn Fn(u32) -> u64,
    /// May tenant `v` be displaced into register `r`? Encodes every
    /// per-register rule Phase 2 applied to it; `false` for any value whose
    /// home must not change (class members, call-spanning values, params).
    reloc_ok: &'a dyn Fn(u32, PhysReg) -> bool,
    /// Holders that pin their callee-saved register (no rename).
    rename_blocked: &'a dyn Fn(u32) -> bool,
    /// FAT intervals of the values that will draw an emitter scratch
    /// register where they live: eligible, homeless, not call-spanning GPR
    /// values (MachInst window vregs).
    scratch_demand: &'a [(u32, u32)],
    /// `CCC_DEBUG_RA`: report every displacement and placement.
    trace: bool,
}

/// One register's tenants (`value`, FAT interval), or `None` when a tenant
/// has no interval (occupancy unknown: the register is never touched).
type IpaOcc = FxHashMap<u8, Option<Vec<(u32, (u32, u32))>>>;

/// Caller-saved registers that must stay free at every program point an
/// IPA-RA placement covers. The emitter's MachInst window allocator and call
/// staging draw scratch from exactly that pool, so the fill must never take
/// its last free register: hot_web_loop's loop had one free caller-saved
/// register (%r11) at its hot point, and a fill that took it cost one copy
/// per iteration (+3.3% Ir) to save two cold-path memory operands.
const IPA_MIN_FREE_SCRATCH: u32 = 1;

fn ipa_disjoint(tenants: &[(u32, (u32, u32))], (s, e): (u32, u32)) -> bool {
    tenants.iter().all(|&(_, (s2, e2))| e < s2 || e2 < s)
}

/// Make IPA register `p` free over every interval of `ivs`, displacing the
/// conflicting tenants as ONE group into a single relocation register they
/// are all allowed in and all disjoint from (the group already shared `p`,
/// so its members stay mutually compatible). Returns `false`, changing
/// nothing, when no such register exists.
fn ipa_make_room(
    assignments: &mut FxHashMap<u32, PhysReg>,
    occ: &mut IpaOcc,
    fill: &IpaFill<'_>,
    p: PhysReg,
    ivs: &[(u32, u32)],
) -> bool {
    let Some(Some(tenants)) = occ.get(&p.0) else {
        return false;
    };
    let (group, rest): (Vec<_>, Vec<_>) = tenants
        .iter()
        .copied()
        .partition(|&(_, iv)| !ivs.iter().all(|&own| ipa_disjoint(&[(0, iv)], own)));
    if group.is_empty() {
        return true;
    }
    let target = fill.reloc_pool.iter().copied().find(|&q| {
        q.0 != p.0
            && occ.get(&q.0).is_some_and(|o| {
                o.as_deref().is_some_and(|qt| {
                    group
                        .iter()
                        .all(|&(v, iv)| (fill.reloc_ok)(v, q) && ipa_disjoint(qt, iv))
                })
            })
    });
    let Some(q) = target else {
        return false;
    };
    if fill.trace {
        eprintln!("[RA-IPA] displace {:?} from r{} to r{}", group, p.0, q.0);
    }
    for &(v, _) in &group {
        assignments.insert(v, q);
    }
    if let Some(Some(qt)) = occ.get_mut(&q.0) {
        qt.extend_from_slice(&group);
    }
    occ.insert(p.0, Some(rest));
    true
}

/// IPA-RA fill (x86-64 `ipa_ra`), run once after every allocation phase and
/// the overlap repair, so it never competes with them for registers.
///
/// 1. Homing: each slotted call-spanning `candidate` (loop-weighted `cost`
///    descending, ties by id) takes the first IPA register that is free over
///    its FAT interval, else the first one that can be freed by displacing
///    its overlapping tenants to another caller-saved register
///    ([`ipa_make_room`]). Every displaced tenant keeps a register, so no
///    value loses its home. Overlap is INCLUSIVE, as in the overlap verifier.
///    A placement is refused where it would leave fewer than
///    [`IPA_MIN_FREE_SCRATCH`] caller-saved registers free at any point it
///    covers (the emitter's scratch supply).
/// 2. Renaming: each callee-saved register R (last-first) moves ALL its
///    holders to the first IPA register that is, or can be made, free over
///    all of them. A whole-register rename keeps every same-register relation
///    (coalesce webs, phi classes) by construction and drops R's push/pop
///    pair. R stays put if a holder is `rename_blocked` or has no interval.
///
/// Returns the IPA registers that received a call-spanning tenant.
fn ipa_fill_preserved_regs(
    assignments: &mut FxHashMap<u32, PhysReg>,
    candidates: &[u32],
    fill: &IpaFill<'_>,
) -> Vec<PhysReg> {
    if fill.ipa_regs.is_empty() {
        return Vec::new();
    }
    let mut occ: IpaOcc = FxHashMap::default();
    for r in fill.ipa_regs.iter().chain(fill.reloc_pool) {
        occ.insert(r.0, Some(Vec::new()));
    }
    let mut sorted: Vec<(u32, PhysReg)> = assignments.iter().map(|(&v, &r)| (v, r)).collect();
    sorted.sort_unstable_by_key(|&(v, _)| v);
    for (v, reg) in sorted {
        if let Some(slot) = occ.get_mut(&reg.0) {
            match (fill.iv_map.get(&v), slot.as_mut()) {
                (Some(&iv), Some(list)) => list.push((v, iv)),
                _ => *slot = None,
            }
        }
    }
    // Free caller-saved registers per program point (inclusive intervals; a
    // register whose occupancy is unknown counts as busy everywhere). A
    // placement of tenants T lowers this by exactly one on T's points and
    // nowhere else: a fresh home turns a free point busy, and a displaced
    // group trades `p` for an equally free `q` except where T now holds `p`.
    let horizon = fill
        .iv_map
        .values()
        .map(|&(_, e)| e as usize + 1)
        .max()
        .unwrap_or(0);
    let mut free_at: Vec<u32> = vec![0; horizon];
    for r in fill.reloc_pool {
        let Some(Some(tenants)) = occ.get(&r.0) else {
            continue;
        };
        let mut busy = vec![false; horizon];
        for &(_, (s, e)) in tenants {
            for b in busy.iter_mut().take(e as usize + 1).skip(s as usize) {
                *b = true;
            }
        }
        for (f, b) in free_at.iter_mut().zip(&busy) {
            *f += u32::from(!*b);
        }
    }
    for &(s, e) in fill.scratch_demand {
        for f in free_at.iter_mut().take(e as usize + 1).skip(s as usize) {
            *f = f.saturating_sub(1);
        }
    }
    let mut used: Vec<PhysReg> = Vec::new();
    let mut ranked: Vec<(u64, u32, (u32, u32))> = candidates
        .iter()
        .filter(|v| !assignments.contains_key(v))
        .filter_map(|&v| fill.iv_map.get(&v).map(|&iv| ((fill.cost)(v), v, iv)))
        .filter(|&(c, _, _)| c > 0)
        .collect();
    ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut place = |assignments: &mut FxHashMap<u32, PhysReg>,
                     occ: &mut IpaOcc,
                     tenants: &[(u32, (u32, u32))]|
     -> Option<PhysReg> {
        let ivs: Vec<(u32, u32)> = tenants.iter().map(|&(_, iv)| iv).collect();
        let mut covered = vec![false; free_at.len()];
        for &(s, e) in &ivs {
            for c in covered.iter_mut().take(e as usize + 1).skip(s as usize) {
                *c = true;
            }
        }
        if covered
            .iter()
            .zip(free_at.iter())
            .any(|(&c, &f)| c && f <= IPA_MIN_FREE_SCRATCH)
        {
            return None;
        }
        let free_now = fill.ipa_regs.iter().copied().find(|p| {
            occ.get(&p.0).is_some_and(|o| {
                o.as_deref()
                    .is_some_and(|t| ivs.iter().all(|&iv| ipa_disjoint(t, iv)))
            })
        });
        let p = match free_now {
            Some(p) => p,
            None => fill
                .ipa_regs
                .iter()
                .copied()
                .find(|&p| ipa_make_room(assignments, occ, fill, p, &ivs))?,
        };
        if fill.trace {
            eprintln!("[RA-IPA] home {:?} in r{}", tenants, p.0);
        }
        for (f, &c) in free_at.iter_mut().zip(&covered) {
            *f -= u32::from(c);
        }
        for &(v, _) in tenants {
            assignments.insert(v, p);
        }
        if let Some(Some(list)) = occ.get_mut(&p.0) {
            list.extend_from_slice(tenants);
        }
        if !used.contains(&p) {
            used.push(p);
        }
        Some(p)
    };
    for (_, v, iv) in ranked {
        place(assignments, &mut occ, &[(v, iv)]);
    }
    for &r in fill.callee_pool.iter().rev() {
        if fill.ipa_regs.iter().any(|x| x.0 == r.0) {
            continue;
        }
        let mut holders: Vec<u32> = assignments
            .iter()
            .filter(|&(_, reg)| reg.0 == r.0)
            .map(|(&v, _)| v)
            .collect();
        if holders.is_empty() || holders.iter().any(|&v| (fill.rename_blocked)(v)) {
            continue;
        }
        holders.sort_unstable();
        let Some(tenants) = holders
            .iter()
            .map(|&v| fill.iv_map.get(&v).map(|&iv| (v, iv)))
            .collect::<Option<Vec<(u32, (u32, u32))>>>()
        else {
            continue;
        };
        place(assignments, &mut occ, &tenants);
    }
    used
}

/// Derive the COLD-CALL XMM SPLIT save set from the FINAL assignments: an
/// XMM register is saved and restored around a cold call point exactly when
/// some value homed in it is live across that call.  Deriving it from the
/// final homes (not from the scan that admitted the value) keeps it exact
/// for copy-web members that inherit a leader's register and after every
/// repair/drop phase.  The point spans `(cp, cp)` reuse the Phase-2b
/// emission (`start <= point <= end`).
/// Save-set compaction (RA-SAVE-SET-COMPACT): fold the WHOLE holder set of a
/// callee-saved register into another callee-saved register that is already
/// saved, when no holder of one overlaps a holder of the other. Each folded
/// register drops a push/pop pair (and its CFI) from the prologue.
///
/// The phases place values with independent occupancy models (Phase 1, the
/// Phase 2c tiers, phi propagation, rehoming), so the final map can name a
/// register whose holders would all fit beside another's -- find_bit's
/// Phase 2c scan homed v53 [34,35] in a fresh %r14 while %rbx (v28 [15,16])
/// was long dead. Placement heuristics cannot fix that without perturbing
/// spill choices (see RA-CALLEE-STICKY); a post-pass changes register names
/// only.
///
/// Soundness: holders of one register already coexist legally (coalesce
/// classes, death-point sharing, segment fills), and moving ALL of them
/// keeps every such relation; the new co-tenants are judged by FAT intervals
/// with INCLUSIVE overlap, stricter than any phase (no touching, no hole
/// sharing), so no two values of different classes can meet. Both registers
/// come from `pool` (the asm-filtered callee-saved list), so call and asm
/// survival are unchanged. A holder without an interval pins its register
/// (neither source nor target). Returns the `(from, to)` folds in order.
fn compact_callee_saved_homes(
    assignments: &mut FxHashMap<u32, PhysReg>,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    pool: &[PhysReg],
) -> Vec<(PhysReg, PhysReg)> {
    let mut regs: Vec<PhysReg> = pool.to_vec();
    regs.sort_unstable_by_key(|r| r.0);
    regs.dedup_by_key(|r| r.0);
    let mut holders: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); regs.len()];
    let mut pinned = vec![false; regs.len()];
    for (&v, &r) in assignments.iter() {
        let Some(i) = regs.iter().position(|p| p.0 == r.0) else {
            continue;
        };
        match iv_map.get(&v) {
            Some(&(s, e)) => holders[i].push((v, s, e)),
            None => pinned[i] = true,
        }
    }
    for h in &mut holders {
        h.sort_unstable();
    }
    let span = |h: &[(u32, u32, u32)]| -> u64 {
        h.iter()
            .map(|&(_, s, e)| u64::from(e.saturating_sub(s)) + 1)
            .sum()
    };
    // Cheapest registers first: few, short holders fit beside others best.
    let mut order: Vec<usize> = (0..regs.len())
        .filter(|&i| !pinned[i] && !holders[i].is_empty())
        .collect();
    order.sort_by_key(|&i| (holders[i].len(), span(&holders[i]), regs[i].0));
    let mut moves = Vec::new();
    for src in order {
        if holders[src].is_empty() {
            continue;
        }
        let fits = |dst: usize| {
            holders[src]
                .iter()
                .all(|&(_, s1, e1)| holders[dst].iter().all(|&(_, s2, e2)| e1 < s2 || e2 < s1))
        };
        let Some(dst) =
            (0..regs.len()).find(|&d| d != src && !pinned[d] && !holders[d].is_empty() && fits(d))
        else {
            continue;
        };
        let moved = std::mem::take(&mut holders[src]);
        for &(v, _, _) in &moved {
            assignments.insert(v, regs[dst]);
        }
        holders[dst].extend(moved);
        moves.push((regs[src], regs[dst]));
    }
    moves
}

/// Turn every cold call an x86-64 XMM home is live across into a
/// caller-save span `(cp, cp)`: the value keeps its register on the hot
/// path and pays a spill/reload only around the cold call, instead of
/// living on the stack for its whole range.  The call's own result
/// register is never restored over.  Disabled by `CCC_NO_COLD_CALL_SPLIT`.
fn add_cold_call_xmm_saves(
    assignments: &FxHashMap<u32, PhysReg>,
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    enabled: bool,
    caller_save_spans: &mut FxHashMap<u8, Vec<(u32, u32)>>,
) {
    if !enabled || liveness.cold_call_points.is_empty() {
        return;
    }
    let mut points: FxHashSet<(u8, u32)> = FxHashSet::default();
    for seg in &liveness.segments {
        let Some(&reg) = assignments.get(&seg.value_id) else {
            continue;
        };
        if !(20..=33).contains(&reg.0) {
            continue;
        }
        let def = iv_map.get(&seg.value_id).map_or(seg.start, |&(s, _)| s);
        for cp in segment_spanned_calls(seg, def, &liveness.call_points) {
            // XMM homes span only cold calls
            // (`collect_cold_only_call_spanning_owners`). Should that ever
            // break, saving around the call is still sound, so fail closed
            // to a save with an unknown result rather than abort.
            let cold = liveness.cold_call_points.binary_search(&cp);
            debug_assert!(cold.is_ok(), "XMM home live across a non-cold call");
            // Never restore over the call's result: its home receives the
            // return value (a merged copy web may also count a member live
            // into the call block), and any value genuinely live across the
            // call in that register would interfere with the result.
            let result = cold
                .ok()
                .and_then(|k| liveness.cold_call_results.get(k).copied().flatten());
            if result == Some(seg.value_id)
                || result
                    .and_then(|r| assignments.get(&r))
                    .is_some_and(|r| r.0 == reg.0)
            {
                continue;
            }
            points.insert((reg.0, cp));
        }
    }
    let mut points: Vec<(u8, u32)> = points.into_iter().collect();
    points.sort_unstable();
    for (reg, cp) in points {
        caller_save_spans.entry(reg).or_default().push((cp, cp));
    }
}

/// Coalesce owners whose every spanned call point is a cold call
/// (`LivenessResult::cold_call_points`).  Only hole-aware (segmented)
/// values qualify: the fat-interval fallback cannot tell which calls a
/// value is really live across.
fn collect_cold_only_call_spanning_owners(
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    coalesce_member_of: &FxHashMap<u32, u32>,
) -> FxHashSet<u32> {
    let owner_of = |value: u32| coalesce_member_of.get(&value).copied().unwrap_or(value);
    let mut cold: FxHashSet<u32> = FxHashSet::default();
    let mut hot: FxHashSet<u32> = FxHashSet::default();
    if liveness.cold_call_points.is_empty() {
        return cold;
    }
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for seg in &liveness.segments {
        segmented.insert(seg.value_id);
        let def = iv_map.get(&seg.value_id).map_or(seg.start, |&(s, _)| s);
        for cp in segment_spanned_calls(seg, def, &liveness.call_points) {
            if liveness.cold_call_points.binary_search(&cp).is_ok() {
                cold.insert(owner_of(seg.value_id));
            } else {
                hot.insert(owner_of(seg.value_id));
            }
        }
    }
    for interval in &liveness.intervals {
        if !segmented.contains(&interval.value_id) {
            hot.insert(owner_of(interval.value_id));
        }
    }
    cold.retain(|v| !hot.contains(v));
    cold
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
    // Skip every duplicate birth-point hazard, then require any remaining
    // hazard to fall within (start, end]. The caller must establish that
    // `start` is a valid birth exemption for this candidate.
    let idx = points.partition_point(|&p| p <= iv.start);
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

/// Compatibility helper for unit tests; production code uses the explicit
/// config-taking variant below.
pub(crate) fn compute_i686_divrem_pairs(
    func: &IrFunction,
    target: DivRemTarget,
) -> I686DivRemPairs {
    compute_i686_divrem_pairs_with_config(func, target, &RaConfig::default())
}

pub(crate) fn compute_i686_divrem_pairs_with_config(
    func: &IrFunction,
    target: DivRemTarget,
    config: &RaConfig,
) -> I686DivRemPairs {
    let mut pairs = I686DivRemPairs {
        tail_dests: FxHashSet::default(),
        head_partners: FxHashMap::default(),
        tail_points: FxHashSet::default(),
        head_point_of_tail: FxHashMap::default(),
    };
    if config.no_ir_divrem {
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

    // Only distinguish zero, one, and multiple definitions. Fusion publishes
    // one head-point mapping per destination, so multi-def dests cannot pair.
    let mut def_count: FxHashMap<u32, u8> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                let count = def_count.entry(dest.0).or_insert(0);
                *count = count.saturating_add(1).min(2);
            }
        }
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
    let flavor = |op: IrBinOp| -> Option<(bool, bool)> {
        // (signed, quotient)
        match op {
            IrBinOp::SDiv => Some((true, true)),
            IrBinOp::SRem => Some((true, false)),
            IrBinOp::UDiv => Some((false, true)),
            IrBinOp::URem => Some((false, false)),
            _ => None,
        }
    };
    let operand_stamp = |operand: &Operand, last_def: &FxHashMap<u32, usize>| -> Option<usize> {
        match operand {
            Operand::Value(value) => last_def.get(&value.0).copied(),
            Operand::Const(_) => None,
        }
    };

    struct DivRemCand<'a> {
        instruction_index: usize,
        ty: &'a IrType,
        lhs: &'a Operand,
        rhs: &'a Operand,
        lhs_stamp: Option<usize>,
        rhs_stamp: Option<usize>,
        barrier_stamp: Option<usize>,
        signed: bool,
        quotient: bool,
        dest: u32,
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        let mut cands: Vec<DivRemCand<'_>> = Vec::new();
        let mut last_def: FxHashMap<u32, usize> = FxHashMap::default();
        let mut barrier_stamp: Option<usize> = None;
        for (ii, inst) in block.instructions.iter().enumerate() {
            // Do not propagate a pairing proof through an instruction that
            // can change the execution context. Ordinary calls are not
            // automatic barriers: early-born tail results are already
            // modeled across calls by `patch_divrem_tail_intervals`.
            let exceptional_barrier = matches!(
                inst,
                Instruction::InlineAsm { .. }
                    | Instruction::NonlocalGotoSave { .. }
                    | Instruction::Intrinsic {
                        op: IntrinsicOp::BuiltinSetjmp
                            | IntrinsicOp::BuiltinLongjmp
                            | IntrinsicOp::DoBuiltinApply,
                        ..
                    }
            ) || crate::backend::liveness::is_returns_twice_call(inst);
            if exceptional_barrier {
                barrier_stamp = Some(ii);
            }
            if let Instruction::BinOp {
                dest,
                op,
                ty,
                lhs,
                rhs,
                ..
            } = inst
            {
                if let Some((signed, quotient)) = flavor(*op) {
                    // i686: constant divisors pair too: the head folds the
                    // pair into ONE magic-number sequence (q in %eax, r in
                    // %edx) or, at -Os, one `divl $imm`-staged division.
                    // Other targets keep the non-constant rule.
                    let constant_rhs_ok =
                        target == DivRemTarget::I686 || !matches!(rhs, Operand::Const(_));
                    if is_gpr32_ty(ty)
                        && constant_rhs_ok
                        && def_count.get(&dest.0).copied() == Some(1)
                    {
                        cands.push(DivRemCand {
                            instruction_index: ii,
                            ty,
                            lhs,
                            rhs,
                            lhs_stamp: operand_stamp(lhs, &last_def),
                            rhs_stamp: operand_stamp(rhs, &last_def),
                            barrier_stamp,
                            signed,
                            quotient,
                            dest: dest.0,
                        });
                    }
                }
            }
            // Input stamps refer to the state before this definition.
            if let Some(dest) = inst.dest() {
                last_def.insert(dest.0, ii);
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
            let mut mate: Option<usize> = None;
            for j in (i + 1)..cands.len() {
                if used[j] {
                    continue;
                }
                let head = &cands[i];
                let tail = &cands[j];
                // OPPOSITE flavours only, same width, same operand
                // incarnations. A same-flavour pair is a CSE miss, not
                // fusion. Identical value-ids after phi-elim do not imply
                // identical incarnations if a redef sits between them.
                if head.ty == tail.ty
                    && head.signed == tail.signed
                    && head.quotient != tail.quotient
                    && head.lhs == tail.lhs
                    && head.rhs == tail.rhs
                    && head.lhs_stamp == tail.lhs_stamp
                    && head.rhs_stamp == tail.rhs_stamp
                    && head.barrier_stamp == tail.barrier_stamp
                {
                    mate = Some(j);
                    break;
                }
            }
            let Some(j) = mate else { continue };
            used[i] = true;
            used[j] = true;
            let head = &cands[i];
            let tail = &cands[j];
            pairs
                .head_partners
                .insert(head.dest, (tail.dest, tail.quotient));
            pairs.tail_dests.insert(tail.dest);
            pairs.tail_points.insert((bi, tail.instruction_index));
            pairs
                .head_point_of_tail
                .insert(tail.dest, block_start[bi] + head.instruction_index as u32);
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
fn patch_divrem_tail_intervals(
    func: &IrFunction,
    liveness: &mut LivenessResult,
    ra_config: &RaConfig,
) {
    let Some(target) = divrem_target_for_current_arch() else {
        return;
    };
    let pairs = compute_i686_divrem_pairs_with_config(func, target, ra_config);
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
/// Compatibility helper for unit tests; production code uses the explicit
/// config-taking variant below.
pub(crate) fn compute_i686_mulacc_chains(func: &IrFunction) -> I686MulAccChains {
    compute_i686_mulacc_chains_with_config(func, &RaConfig::default())
}

pub(crate) fn compute_i686_mulacc_chains_with_config(
    func: &IrFunction,
    config: &RaConfig,
) -> I686MulAccChains {
    let mut out = I686MulAccChains {
        chains: Vec::new(),
        head_of: FxHashMap::default(),
        tail_of: FxHashMap::default(),
    };
    if config.no_mulacc {
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
            |op: &Operand, use_point: u32, _after_head: bool| -> Option<(Operand, Option<u32>)> {
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
fn patch_mulacc_intervals(func: &IrFunction, liveness: &mut LivenessResult, ra_config: &RaConfig) {
    let chains = compute_i686_mulacc_chains_with_config(func, ra_config);
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

/// Shared with the slot census so the "why is this in memory" taxonomy uses
/// the allocator's own predicate instead of a lookalike copy.
pub(crate) fn is_non_gpr_type(ty: &IrType, is_32bit: bool) -> bool {
    ty.is_float()
        || ty.is_long_double()
        || matches!(ty, IrType::I128 | IrType::U128)
        || (is_32bit && matches!(ty, IrType::I64 | IrType::U64))
}

/// Whether a `Cast`'s result can live in a general-purpose register: both
/// sides are GPR types, or it is an integer truncation OUT of a 128-bit
/// value. The truncation's result is an ordinary narrow scalar: every backend
/// routes it through `emit_cast_default` (x86-64 and AArch64 after their
/// narrow/FP fast paths, RISC-V and i686 directly), which loads the pair,
/// narrows the low half and writes the result with the register-aware
/// `emit_store_result`. Refusing it parked every member a two-register
/// struct return carries (`trunc(v)`, `trunc(v >> 64)`) in a stack slot.
/// The single source of truth for both `eligible` and
/// `collect_non_gpr_values`.
fn cast_result_is_gpr(from_ty: IrType, to_ty: IrType, is_32bit: bool) -> bool {
    let wide = |t: IrType| matches!(t, IrType::I128 | IrType::U128);
    let wide_int_trunc = wide(from_ty) && to_ty.is_integer() && !wide(to_ty);
    !is_non_gpr_type(&to_ty, is_32bit) && (wide_int_trunc || !is_non_gpr_type(&from_ty, is_32bit))
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

/// Price the exact set removed by `evict_group`, once per distinct member.
/// Saturation makes overflow lose an optimization instead of wrapping a hot
/// victim into a falsely cheap one. Kept separate for adversarial cost tests.
fn hot_web_eviction_cost(
    victims: &[u32],
    member_of: &FxHashMap<u32, u32>,
    groups: &FxHashMap<u32, Vec<u32>>,
    use_count: &FxHashMap<u32, u64>,
) -> u64 {
    let mut seen = FxHashSet::default();
    let mut total = 0u64;
    for &victim in victims {
        let owner = member_of.get(&victim).copied().unwrap_or(victim);
        let solo = [victim];
        let members = groups.get(&owner).map_or(solo.as_slice(), Vec::as_slice);
        for &member in members {
            if seen.insert(member) {
                total = total.saturating_add(use_count.get(&member).copied().unwrap_or(0));
            }
        }
    }
    total
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
            let total = summed_use_weight(members, use_count);
            if total > r.priority {
                // The cost model needs the same information: a coalesce
                // leader is really read by every member of its web, not
                // just through its own IR use chain.
                let boost = total / r.priority.max(1);
                r.priority = total;
                r.boost_cost(boost);
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

/// Loop weight of the block defining each deferred fold address: the weight
/// at which that GEP's read of its base entered `use_count`.
fn fold_def_block_weights(
    func: &IrFunction,
    transfer: &FxHashMap<u32, Vec<u32>>,
    block_loop_weight: &[u64],
) -> FxHashMap<u32, u64> {
    let moved: FxHashSet<u32> = transfer.values().flatten().copied().collect();
    let mut out = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let weight = block_loop_weight.get(block_idx).copied().unwrap_or(1);
        for inst in &block.instructions {
            if let Some(dest) = inst.dest()
                && moved.contains(&dest.0)
            {
                out.insert(dest.0, weight);
            }
        }
    }
    out
}

/// FOLD-DEFER: drop constant-offset fold addresses from the candidate set
/// while their base is a candidate (see `RegAllocConfig::fold_base_regs`).
///
/// LICM hoists a loop-invariant `GEP p, C` (`&p->field`) into the preheader.
/// Each such address then became a loop-spanning interval carrying the
/// loop-weighted uses of its accesses, while the base `p` — whose register is
/// what the emitter actually reads at a folded access — was weighted only by
/// the preheader GEPs. Five `&optPtr->field` addresses in zstd's
/// `ZSTD_compressBlock_opt2` match-length loop out-ranked `optPtr` itself:
/// the base and all five addresses were spilled, and every price lookup became
/// `movq slot,%rcx; op (%rcx)` where GCC emits `op C(%r12)`. One register for
/// the base replaces every address register; the addresses are phantom
/// pressure whenever the base is homed.
///
/// Deferral is a preference, not a verdict: a base can still lose its
/// register (a call-spanning base competes for the six callee-saved homes,
/// while its addresses may live in a call-free stretch with a caller-saved
/// register to spare). `run_regalloc_and_merge_clobbers_ex` therefore checks
/// every deferred base's final home and re-allocates with the addresses of
/// the bases that failed re-admitted (`RegAllocConfig::fold_readmit`).
///
/// Sound by construction: deferral only removes a candidate. A deferred value
/// is not `never_materialized`, so it keeps a stack home; the emitter folds the
/// access against the base register (the producer emits nothing) or, when the
/// base has no foldable home, materialises the address into that stack home
/// and reads it back — the path every spilled address already takes. The
/// base's interval already reaches every folded access through the
/// `folded_index_uses` hidden reads.
///
/// Returns base -> deferred addresses: the allocator credits the base with
/// their uses (`RangeMetadata::transfer_uses`), since every one of those
/// accesses reads the base's home.
fn defer_const_fold_addresses(
    func: &IrFunction,
    eligible: &mut FxHashSet<u32>,
    readmit: &FxHashSet<u32>,
) -> FxHashMap<u32, Vec<u32>> {
    let mut transfer: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let links = crate::backend::generation::collect_gep_fold_base_links(func);
    if links.is_empty() {
        return transfer;
    }
    let transient = transient_fold_addresses(func, &links);
    for (base, dests) in links {
        if !eligible.contains(&base) {
            continue;
        }
        let moved: Vec<u32> = dests
            .into_iter()
            .filter(|d| !transient.contains(d) && !readmit.contains(d) && eligible.remove(d))
            .collect();
        if !moved.is_empty() {
            transfer.insert(base, moved);
        }
    }
    transfer
}

/// Fold addresses whose deferral relieves no register pressure: every use
/// sits in the GEP's own block and no instruction between the GEP and its
/// last use defines a value (`GEP p, C; load`). Such an address dies exactly
/// where the next value is born, so its candidacy never denies anyone a
/// register, and the emitter folds the access against the base whether or
/// not the address is homed. Deferring it would only reorder the linear
/// scan's arrivals — measured on sha256_transform: the state-word loads'
/// `&state[k]` deferral flipped which loop span won a free register at loop
/// entry and cost a store/reload of the MAJ temporary in the 64-round loop
/// (GCC: zero stack memops there). Deferral keeps its purpose: addresses that
/// live across other definitions or blocks (LICM-hoisted `&p->field`).
fn transient_fold_addresses(func: &IrFunction, links: &FxHashMap<u32, Vec<u32>>) -> FxHashSet<u32> {
    let candidates: FxHashSet<u32> = links.values().flatten().copied().collect();
    // value -> (def block, def index); block-local scan state per candidate.
    let mut def_at: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest()
                && candidates.contains(&dest.0)
            {
                def_at.insert(dest.0, (bi, ii));
            }
        }
    }
    // Last in-block use index (terminator = len) per candidate; any use in
    // another block disqualifies it.
    let mut last_use: FxHashMap<u32, usize> = FxHashMap::default();
    let mut escapes: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        let mut note = |v: u32, at: usize| {
            if let Some(&(db, di)) = def_at.get(&v) {
                // A same-block use at or before the def is a phi operand fed
                // around a single-block loop's backedge: live across the loop.
                if db == bi && at > di {
                    let e = last_use.entry(v).or_insert(at);
                    *e = (*e).max(at);
                } else {
                    escapes.insert(v);
                }
            }
        };
        for (ii, inst) in block.instructions.iter().enumerate() {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    note(v.0, ii);
                }
            });
            for_each_value_use_in_instruction(inst, |v| note(v.0, ii));
        }
        let end = block.instructions.len();
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                note(v.0, end);
            }
        });
    }
    let mut out = FxHashSet::default();
    for (&v, &(bi, di)) in &def_at {
        if escapes.contains(&v) {
            continue;
        }
        let Some(&lu) = last_use.get(&v) else {
            continue;
        };
        let insts = &func.blocks[bi].instructions;
        let between = insts.get(di + 1..lu).unwrap_or(&[]);
        if between.iter().all(|inst| inst.dest().is_none()) {
            out.insert(v);
        }
    }
    out
}

/// Extra allocation rounds for FOLD-DEFER re-admission (see
/// `defer_const_fold_addresses`). One round settles nearly every function: a
/// re-admitted address changes the pressure its base sees, so a second
/// re-admission round is allowed and the rest is left to the stack homes.
const FOLD_READMIT_ROUNDS: usize = 2;

/// `allocate_registers` plus FOLD-DEFER re-admission: a deferred address only
/// pays off when its base ends in a register of `config.fold_base_regs`.
/// Re-admit the addresses of every base that did not and allocate again.
///
/// Terminates: deferral skips `fold_readmit`, so every failed address is new
/// to the set and each extra round strictly grows it (at most one round per
/// deferred address, capped at `FOLD_READMIT_ROUNDS`). Whatever still fails
/// after the cap is sound — the address is materialised in its stack home,
/// the path every spilled address takes — merely unoptimised. A function
/// whose deferred bases all get foldable homes costs exactly one allocation.
pub(crate) fn allocate_registers_with_fold_rounds(
    func: &IrFunction,
    config: &mut RegAllocConfig,
) -> RegAllocResult {
    let mut result = allocate_registers(func, config);
    for _ in 0..FOLD_READMIT_ROUNDS {
        let failed: Vec<u32> = result
            .fold_deferred
            .iter()
            .filter(|(base, _)| {
                !result
                    .assignments
                    .get(base)
                    .is_some_and(|r| config.fold_base_regs.contains(r))
            })
            .flat_map(|(_, moved)| moved.iter().copied())
            .collect();
        if failed.is_empty() {
            break;
        }
        config.fold_readmit.extend(failed);
        result = allocate_registers(func, config);
    }
    result
}

fn bump_folded_index_priority(
    ranges: &mut [crate::backend::live_range::LiveRange],
    folded_index_uses: &FxHashMap<u32, Vec<u32>>,
    safe_homes: &FxHashSet<u32>,
    ra_config: &RaConfig,
) {
    if ra_config.no_index_home {
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
            // Folded index reads are invisible in the IR use chain; scale
            // the position-relative cost by the same structural factor so
            // mode 6 prices them like the multi-read values they are.
            let boost = weight / range.priority.max(1);
            range.priority = weight;
            range.boost_cost(boost);
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
            // A GEP base folded into addressing is read at every folded
            // access, not only at the GEP instructions the IR shows.
            let boost = boosted / r.priority.max(1);
            r.priority = boosted;
            r.boost_cost(boost);
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
    segments: &[LiveInterval],
    eligible: &FxHashSet<u32>,
    param_ref_values: &FxHashSet<u32>,
    ra_config: &RaConfig,
) -> FxHashMap<u32, Vec<u32>> {
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    let mut segments_of: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for segment in segments {
        segments_of
            .entry(segment.value_id)
            .or_default()
            .push((segment.start, segment.end));
    }
    for pieces in segments_of.values_mut() {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_segment_union(pieces, &source);
    }
    let load_def_types = unique_load_def_types(func);
    // (cast dest, cast src) edges where the cast is a bit-preserving no-op on
    // the register: same-width U32<->I32, or a widening of a sub-word load
    // whose register already holds the extended value. For these, dest and src
    // hold IDENTICAL values, so their live ranges may overlap and still share
    // a register (the cast emits nothing) — the strict non-overlap check that
    // guards general Copy coalescing does not apply.
    let mut same_value_edges: FxHashSet<(u32, u32)> = FxHashSet::default();
    let mut coalesce_edges: Vec<(u32, u32)> = Vec::new();
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
    let mut has_calls = false;
    for block in &func.blocks {
        for inst in &block.instructions {
            if matches!(
                inst,
                Instruction::Call { .. } | Instruction::CallIndirect { .. }
            ) {
                has_calls = true;
            }
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
                    coalesce_edges.push((d, s));
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
                    coalesce_edges.push((d, s));
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
        members.sort_by_key(|&m| {
            let (start, end) = iv_map.get(&m).copied().unwrap_or((0, 0));
            (start, end, m)
        });
        let mut accepted: Vec<u32> = Vec::new();
        // Overlap tolerance for a same-value edge is sound only while both
        // endpoints hold identical bits. A single-def pair copies once and
        // keeps the equality for the whole later span. A MULTI-DEF endpoint
        // rewrites the shared home at every later definition — foreign
        // content for any other live member — so it may overlap nothing
        // except its own latch partner (whose single use dies at the copy,
        // the guarded latch_same_value shape).
        let def_le_1 = |v: u32| def_count.get(&v).copied().unwrap_or(0) <= 1;
        // Overlap tolerance is sound only while both endpoints hold
        // IDENTICAL bits for the whole overlapped region. A single-def
        // same-value pair copies once and keeps the equality afterwards, so
        // the edge may tolerate the overlap. A MULTI-DEF endpoint (a
        // loop-latch or phi-transport copy dest) rewrites the shared home at
        // every later definition — foreign content for any member still
        // live across it (arp_create: the v1376/v71 web let v71's def clobber
        // still-live v1376's only home mid-span, leaving it with no
        // slot/remat/acc recovery at its consumer and tripping the
        // fail-closed operand_to_rax gate) — so multi-def members never get
        // overlap tolerance and take their own homes.
        let edge_pair = |m: u32, a: u32| -> bool {
            (same_value_edges.contains(&(m, a)) || same_value_edges.contains(&(a, m)))
                && def_le_1(m)
                && def_le_1(a)
        };
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
                            || edge_pair(m, *a)
                            // Phi-web members on exclusive CFG paths may
                            // share a home. Linearized fat intervals overlap
                            // even then; hole-aware segments are the proof.
                            // Simultaneously-live segments are a miscompile.
                            || (phi_web_class.get(&m).zip(phi_web_class.get(a)).is_some_and(
                                |(cm, ca)| cm == ca,
                            ) && !sorted_coverage_overlaps(
                                &coverage_of_value(m, &segments_of, iv_map),
                                &coverage_of_value(*a, &segments_of, iv_map),
                            ))
                    })
                }),
            };
            if ok {
                accepted.push(m);
            }
        }
        if accepted.len() > 1 {
            if !has_calls {
                let accepted_set: FxHashSet<u32> = accepted.iter().copied().collect();
                let mut comp_parent: FxHashMap<u32, u32> =
                    accepted.iter().map(|&v| (v, v)).collect();
                for &(d, s) in &coalesce_edges {
                    if accepted_set.contains(&d) && accepted_set.contains(&s) {
                        let rd = find(&mut comp_parent, d);
                        let rs = find(&mut comp_parent, s);
                        if rd != rs {
                            comp_parent.insert(rs, rd);
                        }
                    }
                }
                let mut comps: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
                for &v in &accepted {
                    let r = find(&mut comp_parent, v);
                    comps.entry(r).or_default().push(v);
                }
                for (_, comp) in comps {
                    if comp.len() > 1 {
                        let rep = if comp.contains(&leader) {
                            leader
                        } else {
                            comp[0]
                        };
                        result.insert(rep, comp);
                    }
                }
            } else {
                let rep = if accepted.contains(&leader) {
                    leader
                } else {
                    accepted[0]
                };
                result.insert(rep, accepted);
            }
        }
    }

    if ra_config.debug_coalesce {
        eprintln!(
            "[COALESCE] fn={} groups={} members={}",
            func.name,
            result.len(),
            result.values().map(|m| m.len()).sum::<usize>()
        );
        if ra_config.debug_coalesce_members {
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
    let mut out = Vec::with_capacity(liveness.intervals.len());
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for interval in &liveness.intervals {
        let value = interval.value_id;
        if !eligible.contains(&value) || member_of.contains_key(&value) {
            continue;
        }
        let candidate = merged_of.get(&value).copied().unwrap_or(*interval);
        if candidate.start < candidate.end && seen.insert(candidate.value_id) {
            out.push(candidate);
        }
    }
    // Leader may have a degenerate raw interval; members still have range.
    for (&leader, &interval) in merged_of {
        if eligible.contains(&leader)
            && !member_of.contains_key(&leader)
            && interval.start < interval.end
            && seen.insert(leader)
        {
            out.push(interval);
        }
    }
    out.sort_unstable_by_key(|interval| (interval.start, interval.end, interval.value_id));
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
    let mut roots = parent.clone();
    flatten_allocation_classes(&mut roots);
    let mut coverage: FxHashMap<(u8, u32), Vec<(u32, u32)>> = FxHashMap::default();
    let mut segmented: FxHashSet<u32> = FxHashSet::default();
    for segment in &liveness.segments {
        let Some(&reg) = assignments.get(&segment.value_id) else {
            continue;
        };
        segmented.insert(segment.value_id);
        let class = roots
            .get(&segment.value_id)
            .copied()
            .unwrap_or(segment.value_id);
        coverage
            .entry((reg.0, class))
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
        let class = roots
            .get(&interval.value_id)
            .copied()
            .unwrap_or(interval.value_id);
        coverage
            .entry((reg.0, class))
            .or_default()
            .push((interval.start, interval.end));
    }
    let mut spans = Vec::new();
    for ((reg, class), mut pieces) in coverage {
        pieces.sort_unstable();
        let mut normalized = Vec::new();
        insert_segment_union(&mut normalized, &pieces);
        spans.extend(
            normalized
                .into_iter()
                .map(|(start, end)| (reg, start, end, class)),
        );
    }
    overlapping_class_spans(spans)
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
            ipa_used: Vec::new(),
            eligible: FxHashSet::default(),
            fold_deferred: FxHashMap::default(),
            assignments: FxHashMap::default(),
            accumulator_assignments: if has_builtin_setjmp {
                Vec::new()
            } else {
                analyze_accumulator_assignments_with_config(
                    func,
                    config.accumulator_policy,
                    &config.ra_config,
                )
            },
            used_regs: Vec::new(),
            caller_save_spans: FxHashMap::default(),
            liveness: None,
            phi_chain: FxHashMap::default(),
        };
    }

    let is_32bit = crate::common::types::target_is_32bit();
    // Backend-folded operand reads (peeled SIB indices, CMP-REPLAY operands;
    // see RegAllocConfig::folded_index_uses) are ordinary uses to the
    // liveness dataflow, so every interval and hole-aware segment derived
    // below already covers them on every CFG path.
    let no_hidden_reads = FxHashMap::default();
    let hidden_reads = if config.ra_config.no_folded_index_liveness {
        &no_hidden_reads
    } else {
        &config.folded_index_uses
    };
    let mut liveness = compute_live_intervals_with_hidden_reads(func, hidden_reads);
    // DivRem pair tails are physically born at their head's dual-store;
    // extend their intervals before ANY interval map is derived. Without
    // this, homes/slots in the head..tail window get double-assigned.
    patch_divrem_tail_intervals(func, &mut liveness, &config.ra_config);
    // Mul-acc chain tails are born at their head's fused store, and the
    // virtual feeder sources live until the head reads them — same contract.
    patch_mulacc_intervals(func, &mut liveness, &config.ra_config);
    let iv_map = interval_map(&liveness);
    let call_points = &liveness.call_points;

    let arm_fp_pool =
        config.xmm_regs.first().is_some_and(|r| r.0 == 40) && !config.ra_config.no_vecreg;
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
                    if cast_result_is_gpr(*from_ty, *to_ty, is_32bit) {
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
                    let count = use_count.entry(v.0).or_insert(0);
                    *count = count.saturating_add(weight);
                    let entry = use_loop_depth.entry(v.0).or_insert(0);
                    *entry = (*entry).max(depth);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                let count = use_count.entry(v.0).or_insert(0);
                *count = count.saturating_add(weight);
                let entry = use_loop_depth.entry(v.0).or_insert(0);
                *entry = (*entry).max(depth);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                let count = use_count.entry(v.0).or_insert(0);
                *count = count.saturating_add(weight);
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
        if config.ra_config.no_index_home || !safe_folded_index_homes.contains(v) {
            eligible.remove(v);
        }
    }

    let x86_ordered_param_copies = !is_32bit
        && config.available_regs.iter().any(|r| r.0 == 1)
        && config.caller_saved_regs.iter().any(|r| r.0 == 10)
        && x86_param_caller_homes_safe_with_config(func, &config.ra_config);
    // riscv64: same lever for the a0–a7 pool.  Param homes are pinned to
    // their incoming registers by the ABI hints, so the "ordered copies"
    // degenerate to zero instructions per ParamRef.
    let riscv_ordered_param_copies = config.available_regs.iter().any(|r| r.0 == 11)
        && config.caller_saved_regs.iter().any(|r| r.0 == 12)
        && riscv_param_caller_homes_safe_with_config(func, &config.ra_config);
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

    let all_phi_pairs = detect_phi_coalesce_groups_with_config(func, &liveness, &config.ra_config);
    let mut phi_coalesce: Vec<PhiCoalesceCandidate> = Vec::new();
    if !config.ra_config.no_phi_coalesce {
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
    //
    // A source with consumers other than the latch copy must likewise keep
    // its own home: apply_phi_coalesce_assignments only hands the phi's
    // register over when no *other* holder of that register overlaps the
    // source's interval. When that veto fires, a home-less source degrades
    // to slot traffic on both sides of the backedge (store at the def,
    // reload in the latch — `adler_split`'s a-chain regressed exactly this
    // way once the FP gate came off). Homed, the same veto leaves the latch
    // copy as a plain register move, and when the veto does not fire the
    // apply phase moves the source onto the phi's register and the copy
    // vanishes. A source read only by the latch copy stays home-less: the
    // apply phase either gives it the phi's register or the def spills, and
    // no other consumer can observe the difference.
    //
    // One CFG for the whole candidate loop: the caller-saved window check
    // needs the predecessor sets, and deriving them inside the check turned
    // a linear scan into O(candidates x (V+E)) on every function.
    let mut cfg_preds: Option<analysis::FlatAdj> = None;
    for candidate in &phi_coalesce {
        let src_has_other_consumers = uses_excluding(
            func,
            candidate.backedge_src,
            candidate.block_idx,
            candidate.copy_idx,
        ) > 0;
        let preds = cfg_preds.get_or_insert_with(|| {
            let labels = analysis::build_label_map(func);
            analysis::build_cfg(func, &labels).0
        });
        if !phi_window_clobbers_caller_saved(func, candidate, preds) && !src_has_other_consumers {
            if config.ra_config.debug_phi_coalesce {
                eprintln!(
                    "[PHI_COALESCE] fn={} HOMELESS src=v{} (dest=v{} block={} copy_idx={})",
                    func.name,
                    candidate.backedge_src,
                    candidate.phi_dest,
                    candidate.block_idx,
                    candidate.copy_idx
                );
            }
            eligible.remove(&candidate.backedge_src);
        }
    }

    let coalesce_groups: FxHashMap<u32, Vec<u32>> = if !config.ra_config.no_coalesce {
        build_coalesce_groups(
            func,
            &iv_map,
            &liveness.segments,
            &eligible,
            &param_ref_values,
            &config.ra_config,
        )
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
    let mut riscv_entry_guard: FxHashSet<u32> = if config.available_regs.iter().any(|r| r.0 == 11)
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
    propagate_member_restrictions(&mut riscv_entry_guard, &coalesce_member_of);

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
    let call_spanning: FxHashSet<u32> =
        collect_call_spanning_owners(&liveness, &iv_map, &coalesce_member_of, call_points);
    // The same two facts with hidden call SEQUENCES (dynamic TLS on AArch64/
    // RISC-V, `LivenessResult::sequence_call_points`) removed. `call_spanning`
    // stays the ELIGIBILITY fact: every caller-saved filter below keeps it, so
    // no caller-saved home spans a TLS `blr`/`call`. The `real_*` facts drive
    // only the spill-PRIORITY decisions (Phase 1 promotion, leaf detection,
    // Phase 2c admission), which then see the function exactly as when the
    // sequence is not a call: values spanning only a TLS access compete with
    // the hot parameters for the callee-saved pool instead of preempting it.
    // Identical (borrowed) on every target without call sequences.
    let real_call_points: std::borrow::Cow<'_, [u32]> = if liveness.sequence_call_points.is_empty()
    {
        std::borrow::Cow::Borrowed(call_points.as_slice())
    } else {
        std::borrow::Cow::Owned(
            call_points
                .iter()
                .copied()
                .filter(|p| liveness.sequence_call_points.binary_search(p).is_err())
                .collect(),
        )
    };
    let real_spanning: std::borrow::Cow<'_, FxHashSet<u32>> =
        if liveness.sequence_call_points.is_empty() {
            std::borrow::Cow::Borrowed(&call_spanning)
        } else {
            std::borrow::Cow::Owned(collect_call_spanning_owners(
                &liveness,
                &iv_map,
                &coalesce_member_of,
                &real_call_points,
            ))
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

    let fold_use_transfer = if !config.fold_base_regs.is_empty()
        && !config.ra_config.no_fold_addr_defer
        && !config.ra_config.no_folded_index_liveness
    {
        defer_const_fold_addresses(func, &mut eligible, &config.fold_readmit)
    } else {
        FxHashMap::default()
    };
    // The deferred addresses' accesses read the base's register: they are
    // the base's uses for Phase-1 candidacy (`hot_loop_home`) too. The use
    // MOVES rather than adds: a folded GEP emits nothing, so its own read of
    // the base (already in `use_count`, at the GEP block's weight) is
    // replaced by the accesses' reads. Adding instead double-counted every
    // adjacent `GEP p, C` + access pair and inflated cold bases.
    if !fold_use_transfer.is_empty() {
        let def_weight = fold_def_block_weights(func, &fold_use_transfer, &block_loop_weight);
        for (base, moved) in &fold_use_transfer {
            let mut count = 0u64;
            let mut gep_reads = 0u64;
            let mut depth = 0;
            for v in moved {
                let uses = use_count.get(v).copied().unwrap_or(0);
                if uses == 0 {
                    continue;
                }
                count = count.saturating_add(uses);
                gep_reads = gep_reads.saturating_add(def_weight.get(v).copied().unwrap_or(0));
                depth = depth.max(use_loop_depth.get(v).copied().unwrap_or(0));
            }
            let c = use_count.entry(*base).or_insert(0);
            *c = c.saturating_sub(gep_reads).saturating_add(count);
            let d = use_loop_depth.entry(*base).or_insert(0);
            *d = (*d).max(depth);
        }
    }
    let scan_ivs =
        collect_gpr_scan_intervals(&liveness, &eligible, &merged_of, &coalesce_member_of);
    // (phi_dest, backedge_src) pairs for the recurrence sight in
    // `mark_loop_spanning`: the HOMELESS exclusion removes latch sources
    // from the coalesce webs precisely for the recurrence shapes whose
    // increment consumes the web, so the pairs must be fed directly.
    let sparse_call_state = hot_web::has_sparse_mask_comparison(func);
    let call_loop_state = if sparse_call_state
        && crate::common::types::target_elf_machine() == 62
        && config.available_regs.iter().all(|r| (1..=6).contains(&r.0))
    {
        hot_web::loop_carried_copy_dests(func, &liveness)
            .into_iter()
            .filter(|v| call_spanning.contains(v))
            .collect::<FxHashSet<_>>()
    } else {
        FxHashSet::default()
    };
    let mut phi_backedges: Vec<(u32, u32)> = phi_coalesce
        .iter()
        .map(|c| (c.phi_dest, c.backedge_src))
        .collect();
    // Recurrence sight is not a coalescing relation. Conditional backedges
    // that cannot share a register still carry hot state across loop calls.
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Instruction::Copy {
            dest,
            src: Operand::Value(src),
        } = inst
            && call_loop_state.contains(&dest.0)
        {
            phi_backedges.push((dest.0, src.0));
        }
    }
    let build_gpr_ranges = |intervals: &[LiveInterval]| {
        let (mut ranges, range_meta) = live_range::build_live_ranges_with_config_and_meta(
            intervals,
            &liveness.block_loop_depth,
            func,
            &config.ra_config,
            Some(&fold_use_transfer),
        );
        apply_physical_reg_hints(&mut ranges, &config.reg_hints);
        bump_folded_index_priority(
            &mut ranges,
            &config.folded_index_uses,
            &safe_folded_index_homes,
            &config.ra_config,
        );
        // NOTE on the policy boosts: the loop-span admission cap prices
        // candidates with the ranges' own weighted-use costs, and neither
        // the GEP-base nor the coalesce-web boost is applied here. Both
        // boosts change `priority`, which is the scan's secondary SORT
        // KEY — inflating webs or bases in the main waves reorders the
        // whole allocation (measured: expat -30%, adler32 -23%,
        // arith_loop -12% from the GEP-base boost alone; sha256 -56% from
        // the coalesce boost), and every one of those losses dwarfs the
        // cap's wins. The 2c leftover phase keeps its own copies, where
        // the re-ranking is confined to spare registers.
        // RA-05: hole-aware coverage for the scan's interference tests.
        // Values without segment data keep their fat semantics.
        attach_scan_segments(&mut ranges, &liveness, &coalesce_member_of);
        // RA-PRESSURE-1: flag ranges whose fat envelope covers a whole
        // natural loop — they hold their register for the entire body on
        // every iteration and are subject to the admission cap. The
        // member map carries the web-wide in-loop-use flag: a leader's
        // own `uses` under-count a phi web exactly the way its priority
        // does. A merged member owns no range, so its in-extent uses are
        // taken from the same `RangeMetadata::uses` these ranges were cut
        // from — one numbering, and terminator uses included.
        live_range::mark_loop_spanning_with_copy_policy(
            &mut ranges,
            &liveness.loop_extents,
            &coalesce_member_of,
            func,
            &range_meta.uses,
            !config.ra_config.no_web_inloop_use,
            &phi_backedges,
            !call_loop_state.is_empty(),
        );
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
    //
    // RA-23 (loop-leaf extension): the same argument holds for a function
    // whose call points all sit OUTSIDE every loop — `memset` at entry
    // before the sieve, a `printf` after the loop nest, an `abort` in a
    // cold depth-0 tail. No hot loop value spans such a call (a spanning
    // value is `call_spanning` and stays in Phase 1 regardless), so the
    // Phase-1 promotion again only converts a free volatile home into a
    // push/pop pair. Measured on the benchmark corpus at -O2: sieve
    // `count_primes` 5 -> 0 pushes, matmul 6 -> 0, glibc memcmp 6 -> 2,
    // with the loop bodies unchanged. Calls INSIDE a loop keep the boot-
    // corpus behaviour (cmdline_find_option: the state machine wants a
    // dedicated home because the in-loop call fragments the volatile pool).
    // A/B: `CCC_LEAF_STRICT_CALL_FREE=1` restores the call-free-only rule.
    let calls_only_outside_loops = !real_call_points.is_empty()
        && !config.ra_config.leaf_strict_call_free
        && real_call_points.iter().all(|&p| {
            liveness
                .block_starts
                .iter()
                .zip(liveness.block_ends.iter())
                .position(|(&s, &e)| s <= p && p <= e)
                .is_some_and(|b| liveness.block_loop_depth.get(b).copied().unwrap_or(1) == 0)
        });
    let leaf_prefers_caller_saved = config.leaf_caller_saved_homes
        && (real_call_points.is_empty() || calls_only_outside_loops)
        && !config.ra_config.no_leaf_caller_home;
    let hot_loop_home = |iv: &LiveInterval| -> bool {
        if config.ra_config.no_hot_loop || leaf_prefers_caller_saved {
            return false;
        }
        if real_spanning.contains(&iv.value_id) {
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
    // Phase 1 is two waves, in the order the module doc states: values that
    // are live across a call *must* have a callee-saved home (they have no
    // alternative), and the hot-loop promotion below is an optimisation that
    // may only spend what the first wave leaves.  Merging the two into one
    // scan let an optional candidate outrank a mandatory one in the shared
    // `priority` order, and the mandatory one then paid a stack slot reloaded
    // on every use inside its loop.  Measured on
    // tests/benchmark/programs/hot_web_loop.c (one call inside a 40M-trip
    // loop, six callee-saved registers): the four loop-carried accumulators
    // lost their homes to four loop-invariant cold-path constants, so the hot
    // body carried 4 stores + 6 reloads per iteration and ran 1.67x slower
    // than gcc; with the waves separated the impossible-to-avoid values win
    // and the cold constants reload only on the branch that reads them.
    let waves = !config.ra_config.no_p1_waves;
    let phase1_intervals: Vec<LiveInterval> = scan_ivs
        .iter()
        .copied()
        .filter(|iv| real_spanning.contains(&iv.value_id) || hot_loop_home(iv))
        .collect();
    // ONE build over the whole phase-1 worklist, then the split.  The
    // per-range measurements (`mark_loop_spanning`: the loop's block-local
    // reserve, its cost bar, the total span count the admission cap compares
    // against) describe the FUNCTION, so they must be taken over the same set
    // of ranges they were always taken over; building each wave separately
    // would silently re-derive them from a subset.
    let mut phase1_ranges = build_gpr_ranges(&phase1_intervals);
    if config.ra_config.debug_ra_phases {
        let mut rows: Vec<(u32, u32, u32, u64, u32, bool, usize)> = phase1_ranges
            .iter()
            .map(|r| {
                (
                    r.value_id,
                    r.start,
                    r.end,
                    r.priority,
                    r.uses.len() as u32,
                    r.spans_loop,
                    r.segments.len(),
                )
            })
            .collect();
        rows.sort_by_key(|r| r.0);
        for (id, start, end, prio, uses, spans, nseg) in rows {
            let hint = if call_spanning.contains(&id) {
                "spanning"
            } else {
                "optional"
            };
            eprintln!(
                "[RA-P1C] fn={} v={} [{},{}) uses={} priority={} spans_loop={} segments={} {}",
                func.name, id, start, end, uses, prio, spans, nseg, hint
            );
        }
    }
    bump_coalesce_group_priority(&mut phase1_ranges, &coalesce_groups, &use_count);
    bump_gep_base_priority(&mut phase1_ranges, &liveness);
    // Wave one: values a call point cuts through.  They have no alternative
    // home, so they are allocated first, against the whole pool.
    let (mandatory, optional): (Vec<_>, Vec<_>) = if waves {
        phase1_ranges
            .into_iter()
            .partition(|r| call_spanning.contains(&r.value_id))
    } else {
        (phase1_ranges, Vec::new())
    };
    // An over-subscribed mandatory wave has one register per winner to hand
    // out and slots for the rest, so the only decision is WHO wins. Ranking
    // that decision by the cost model (the cost model sorts the worklist
    // instead of relying on source order) was tried, measured at 5 % on
    // `hot_web_loop`, and removed: the scan consults the ACTIVE set at the
    // incoming range's start, so a reordered worklist admits a later range
    // first, parks a register on an interval that has not begun, and demotes
    // an earlier range to a slot it does not have to share
    // (`tests/regression/check_hot_loop_register_homes.sh`). The decision
    // belongs in the victim selection, which is order-independent.
    let mut allocator = LinearScanAllocator::new_with_config(
        mandatory,
        config.available_regs.clone(),
        &config.ra_config,
    );
    // Reuse an already-touched register before opening a fresh one: on this
    // pool each fresh callee-saved register costs a push/pop pair per call,
    // while rotation's false-dependency avoidance is moot under renaming
    // (benchmark programs: 333 -> 320 pushes at -O2, Ir neutral).
    allocator.prefer_touched = true;
    allocator.run();
    let mut assignments = allocator.assignments;
    // Wave two: the hot-loop promotion, which is an optimisation and may only
    // spend what wave one left.  A register a mandatory value claimed stays
    // out of reach even where the promotion would have fitted into one of its
    // holes; that is the price of the strict order, and it costs nothing when
    // the mandatory wave exhausts the pool -- the interesting case -- while a
    // function whose loops contain no call keeps the whole pool, exactly as
    // before.
    let mut phase1_assigned_optional = 0usize;
    if !optional.is_empty() {
        let claimed: FxHashSet<PhysReg> = assignments.values().copied().collect();
        let spare: Vec<PhysReg> = config
            .available_regs
            .iter()
            .copied()
            .filter(|r| !claimed.contains(r))
            .collect();
        if !spare.is_empty() {
            let mut optional_allocator =
                LinearScanAllocator::new_with_config(optional, spare, &config.ra_config);
            optional_allocator.run();
            phase1_assigned_optional = optional_allocator.assignments.len();
            assignments.extend(optional_allocator.assignments);
        }
    }
    if config.ra_config.debug_ra_phases {
        let mut ids: Vec<u32> = assignments.keys().copied().collect();
        ids.sort_unstable();
        eprintln!(
            "[RA-P1] fn={} pool={:?} candidates={} assigned={:?} (optional wave {})",
            func.name,
            config.available_regs,
            phase1_intervals.len(),
            ids,
            phase1_assigned_optional
        );
    }

    let mut used_regs_set: FxHashSet<u8> = FxHashSet::default();
    for &reg in assignments.values() {
        used_regs_set.insert(reg.0);
    }
    let mut caller_used_regs_set: FxHashSet<u8> = FxHashSet::default();

    // Phase 2: caller-saved for non-spanning leftovers (remats welcome here).
    if !config.caller_saved_regs.is_empty() {
        let i686_pool = i686_scratch_pool(config);
        let hazards: Option<(Vec<u32>, Vec<u32>)> = if i686_pool {
            Some(collect_i686_scratch_hazard_points(
                func,
                &non_gpr_values,
                &FxHashSet::default(),
                &config.ra_config,
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
            if config.ra_config.debug_ra_intervals {
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
                if config.ra_config.debug_ra_intervals {
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
                let mut alloc =
                    LinearScanAllocator::new_with_config(ranges, vec![reg], &config.ra_config);
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
                    let mut caller_allocator = LinearScanAllocator::new_with_config(
                        phase2_ranges,
                        config.caller_saved_regs.clone(),
                        &config.ra_config,
                    );
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
                    let mut alloc = LinearScanAllocator::new_with_config(
                        ranges,
                        no_arg_no_indirect_pool,
                        &config.ra_config,
                    );
                    alloc.run();
                    for (vid, reg) in &alloc.assignments {
                        assignments.insert(*vid, *reg);
                        caller_used_regs_set.insert(reg.0);
                        if let Some((start, end)) =
                            allocation_owner_bounds(*vid, &merged_of, &iv_map)
                        {
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

                if config.ra_config.debug_ra_intervals {
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
                    let mut alloc = LinearScanAllocator::new_with_config(
                        ranges,
                        no_indirect_pool,
                        &config.ra_config,
                    );
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

                if config.ra_config.debug_ra_intervals {
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
                    let mut alloc = LinearScanAllocator::new_with_config(
                        ranges,
                        no_arg_pool,
                        &config.ra_config,
                    );
                    alloc.run_with_seed(&seeded);
                    for (vid, reg) in &alloc.assignments {
                        assignments.insert(*vid, *reg);
                        caller_used_regs_set.insert(reg.0);
                        if let Some((start, end)) =
                            allocation_owner_bounds(*vid, &merged_of, &iv_map)
                        {
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

                if config.ra_config.debug_ra_intervals {
                    let ids: Vec<(u32, u8)> = w3
                        .iter()
                        .filter(|iv| assignments.contains_key(&iv.value_id))
                        .map(|iv| (iv.value_id, assignments[&iv.value_id].0))
                        .collect();
                    eprintln!("[RA-W3] fn={} homes={:?}", func.name, ids);
                }
                // Wave 4: the rest (non-call-args and direct arg-0 values) —
                // full caller-saved pool.
                if config.ra_config.debug_ra_intervals {
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
                    let mut alloc = LinearScanAllocator::new_with_config(
                        ranges,
                        config.caller_saved_regs.clone(),
                        &config.ra_config,
                    );
                    alloc.run_with_seed(&seeded);
                    for (vid, reg) in alloc.assignments {
                        assignments.insert(vid, reg);
                        caller_used_regs_set.insert(reg.0);
                    }
                    if config.ra_config.debug_ra_intervals {
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

    // Phase 2-x64 (position-aware %rdx admission). The historical model
    // dropped %rdx from the caller-saved pool for the WHOLE function
    // whenever ANY instruction clobbers it — one cold `a % b` in a tail
    // block cost the register supply of the entire hot loop (lz4: a single
    // post-compression UDiv evicted %rdx while the match-search loop
    // spilled long-lived pointers). The clobbers are position-local: %rdx
    // is exactly as safe as any other caller-saved register for values
    // whose live ranges provably avoid every clobber point, so this wave
    // (mirroring the i686 ecx/edx hazard waves) recovers the register for
    // exactly those values instead of leaving it idle.
    //
    // Ordering — strictly AFTER the general Phase-2 waves and BEFORE
    // Phase 2c: %rdx homes are excluded from the const-offset GEP folds
    // (`const_offset_fold_reg_base_ok` refuses PhysReg 10|16 bases) and
    // from the fold-eligible register classes the general waves hand out,
    // so %rdx must be an OVERFLOW net (only values the 6-register general
    // waves left unhomed), never a first pick — running it first measurably
    // stole fold-eligible homes (lz4 -3%). Before 2c because 2c's
    // callee-saved overflow costs a push/pop pair per register while a
    // caller-saved %rdx home is free.
    //
    //   * pool membership: only when %rdx is NOT already in the general
    //     pool — a clobber-free body keeps the exact historical
    //     single-wave allocation (and the param-home preference order), so
    //     this wave changes codegen ONLY for functions whose %rdx was
    //     previously excluded AND whose leftovers survive the general
    //     waves;
    //   * candidacy: `base_ok` (no home yet, not call-spanning,
    //     param/riscv guards) PLUS `later_arg_values` exclusion (a value
    //     consumed as a call argument at index >= 1 can be read after an
    //     earlier argument's staging wrote %rdx — the contract the general
    //     waves enforce via `no_arg_pool`) PLUS
    //     `!overlaps_inclusive_skip_birth(iv, clobber_points)`: a value
    //     live across a clobber point (inclusive of the point where the
    //     clobbering instruction still READS it, e.g. a divisor staged
    //     before `cqo` zeroes %edx) is refused, while a value BORN at the
    //     point (the div/rem result itself) is admitted — the optimal
    //     shape, the remainder is produced in %rdx;
    //   * wide bodies (I128/U128 ops anywhere) keep the whole-function
    //     exclusion: their emitters flow values through the %rax:%rdx
    //     accumulator pair and were never production-exercised with
    //     %rdx-homed values (the pool gate always excluded %rdx there),
    //     so point-local emitter completeness is asserted, not proven,
    //     for that class. Values crossing i128 helper-call divisions are
    //     doubly excluded — those are call points.
    if !config.ra_config.no_rdx_hazard
        && crate::common::types::target_elf_machine() == crate::backend::elf::EM_X86_64
        && !x86_body_has_wide_ops(func)
        && !config.caller_saved_regs.iter().any(|r| r.0 == 16)
    {
        let rdx_clobbers = collect_x64_rdx_clobber_points(func);
        // Fused div/rem pair tails emit no code at their own IR point (the
        // HEAD stored both results earlier), so a tail dest's modeled
        // interval starts at the tail while the value is physically resident
        // in %rdx from the head onward — another %rdx clobber strictly
        // between head and tail (interleaved pairs) would corrupt it
        // invisibly to the hazard filter. The accumulator analysis excludes
        // tails for the same physical-liveness reason; so does the wave.
        let divrem_tail_dests: FxHashSet<u32> = match divrem_target_for_current_arch() {
            Some(t) => compute_i686_divrem_pairs_with_config(func, t, &config.ra_config).tail_dests,
            None => FxHashSet::default(),
        };
        if !rdx_clobbers.is_empty() {
            // The Phase-2 `base_ok` gates, inlined (the closure is scoped to
            // the caller-saved block above): no home yet, not call-spanning,
            // param restrictions honoured. Note on fold eligibility: a
            // %rdx home is refused by `const_offset_fold_reg_base_ok`
            // (PhysReg 10|16 exclusion) but accepted by the indexed fold
            // (`can_indexed_addr_fold` consults register homes generically),
            // so GEP bases trade const-offset folding for the free register —
            // the documented ordering choice of this wave, measured neutral
            // on the corpus.
            let intervals: Vec<LiveInterval> = scan_ivs
                .iter()
                .copied()
                .filter(|iv| {
                    !assignments.contains_key(&iv.value_id)
                        && !call_spanning.contains(&iv.value_id)
                        && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                        && !riscv_entry_guard.contains(&iv.value_id)
                        && !later_arg_values.contains(&iv.value_id)
                        && !divrem_tail_dests.contains(&iv.value_id)
                        && !overlaps_inclusive_skip_birth(iv, &rdx_clobbers)
                })
                .collect();
            if config.ra_config.debug_ra_intervals {
                let cands: Vec<u32> = intervals.iter().map(|iv| iv.value_id).collect();
                eprintln!(
                    "[RA-P2X] fn={} rdx-hazard clobbers={} candidates={:?}",
                    func.name,
                    rdx_clobbers.len(),
                    cands
                );
            }
            if !intervals.is_empty() {
                let ranges = build_gpr_ranges(&intervals);
                let mut alloc = LinearScanAllocator::new_with_config(
                    ranges,
                    vec![PhysReg(16)],
                    &config.ra_config,
                );
                alloc.run();
                for (vid, r) in alloc.assignments {
                    assignments.insert(vid, r);
                    caller_used_regs_set.insert(r.0);
                }
            }
        }
    }

    // Phase 2-x64w (position-aware r8/r9/rdi/rsi admission for wide bodies).
    // The prologue withdraws these four from the pool for the WHOLE function
    // as soon as one 128-bit instruction exists, because the wide emitters
    // use them as scratch. Those clobbers are point-local (see
    // `collect_x64_wide_hazard_points`), so values whose live ranges avoid
    // every wide point get them back. A function returning a 9..=16-byte
    // aggregate packs it as I128 at its return, and an un-inlined helper of
    // that kind used to run its whole body on r10/r11 plus callee-saved
    // pushes. Same candidacy as the general Phase-2 waves (no home yet, not
    // call-spanning, param guards, `later_arg_values` kept off the argument
    // registers), plus fused div/rem tails, whose value is physically
    // resident from the head on (the `%rdx` wave's reason). Overflow-only
    // like the `%rdx` wave: it runs after the general waves, so it cannot
    // displace a fold-eligible r10/r11 home.
    if crate::common::types::target_elf_machine() == crate::backend::elf::EM_X86_64
        && !config
            .caller_saved_regs
            .iter()
            .any(|r| matches!(r.0, 12 | 13 | 14 | 15))
        && x86_body_has_i128_ops(func)
    {
        let mut pool = vec![PhysReg(12), PhysReg(13), PhysReg(15)];
        if !x86_body_fixed_scratch(func).rdi {
            pool.push(PhysReg(14));
        }
        let hazards = collect_x64_wide_hazard_points(func);
        let divrem_tail_dests: FxHashSet<u32> = match divrem_target_for_current_arch() {
            Some(t) => compute_i686_divrem_pairs_with_config(func, t, &config.ra_config).tail_dests,
            None => FxHashSet::default(),
        };
        let intervals: Vec<LiveInterval> = scan_ivs
            .iter()
            .copied()
            .filter(|iv| {
                !assignments.contains_key(&iv.value_id)
                    && !call_spanning.contains(&iv.value_id)
                    && (ordered_param_homes || !param_restricted.contains(&iv.value_id))
                    && !riscv_entry_guard.contains(&iv.value_id)
                    && !later_arg_values.contains(&iv.value_id)
                    && !divrem_tail_dests.contains(&iv.value_id)
                    && !overlaps_inclusive(iv, &hazards)
            })
            .collect();
        if config.ra_config.debug_ra_intervals {
            let cands: Vec<u32> = intervals.iter().map(|iv| iv.value_id).collect();
            eprintln!(
                "[RA-P2W] fn={} wide-hazard points={} candidates={:?}",
                func.name,
                hazards.len(),
                cands
            );
        }
        if !intervals.is_empty() {
            let ranges = build_gpr_ranges(&intervals);
            let mut alloc = LinearScanAllocator::new_with_config(ranges, pool, &config.ra_config);
            alloc.run();
            for (vid, r) in alloc.assignments {
                assignments.insert(vid, r);
                caller_used_regs_set.insert(r.0);
            }
        }
    }

    // Phase 2c: leftover callee-saved for call-free overflow. Callee-saved
    // homes survive a call sequence, so a value spanning only TLS sequences
    // (`real_spanning` excludes it from Phase 1) competes here.
    {
        let mut phase2c_intervals: Vec<LiveInterval> = scan_ivs
            .iter()
            .copied()
            .filter(|iv| {
                !assignments.contains_key(&iv.value_id) && !real_spanning.contains(&iv.value_id)
            })
            .collect();
        // Tier 1: overflow takes the callee-saved registers no earlier phase
        // used (each costs a push/pop pair), ranked by the full cost model.
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
                // NOTE (2026-09-08): running this scan with the
                // span-pressure valve off was tried and REVERTED: without
                // the valve the first-admitted spans clog the pool (the
                // ordinary cost model never outbids a span's remaining
                // cost, so nothing evicts them) and every short behind
                // them spills — chacha20_core went from 33 to 64 spills.
                // The valve is load-bearing in every wave; see the
                // ChaCha20 section of the engineering follow-up for the
                // measurement record and the real roadmap (splitting).
                let mut spill_allocator = LinearScanAllocator::new_with_config(
                    phase2c_ranges,
                    free_callee,
                    &config.ra_config,
                );
                // No `prefer_touched` here (measured, RA-CALLEE-STICKY):
                // this scan cannot evict, so which overflow interval spills
                // follows placement order; sticky placement spilled zstd's
                // hot v26 where rotation spilled the cold v158 (+4% Ir).
                spill_allocator.run();
                for (vid, reg) in spill_allocator.assignments {
                    assignments.insert(vid, reg);
                    used_regs_set.insert(reg.0);
                }
            }
        }
        phase2c_intervals.retain(|iv| !assignments.contains_key(&iv.value_id));
        // Tier 2 (RA-PHASE2C-SAVED-REUSE): what the fresh scan left
        // unhomed competes for the free stretches of every callee-saved
        // register the prologue now saves anyway. The tier only ADDS homes,
        // so no value the fresh scan homed loses its register.
        //
        // Ranking is load-bearing: the tier is a no-eviction fill, so
        // whoever is placed first keeps the stretch. A start-ordered linear
        // scan here (S81) was sticky placement in another form
        // (RA-CALLEE-STICKY): saved-register stretches went to whichever
        // overflow interval started first, regardless of loop weight.
        // zstd's ZSTD_decompressSequences_bmi2 spilled its three
        // per-sequence nbBits bytes and materialized two table-entry
        // addresses into a stack slot: +3.0% Ir in that function at every
        // decompression level (d-3: 23.53M vs 22.86M with the tier off).
        // Candidates are therefore ranked like Phase 2f's segment fill,
        // hottest web first. The tier stays because 2f skips Copy webs:
        // without it strlen_bench's outer loop counter (a copy-coalesced
        // loop web) round-trips the stack every iteration (+675K Ir).
        // Interference uses fat group-merged envelopes. Phase 2f's
        // hole-aware segments were measured and rejected here: they pack
        // more webs into early holes and lost on zstd (c-19 +1.4%, c-1
        // +0.9% Ir vs fat).
        // The case that motivated the tier -- linux_find_bit paying a fresh
        // %r15 save per call for a 4-point temp while rbx/r12/r13 sat idle
        // -- is a register-NAME problem, which save-set compaction
        // (`compact_callee_saved_homes`, after all phases) folds without
        // touching any spill choice.
        if !phase2c_intervals.is_empty() && !config.ra_config.no_phase2c_reuse {
            let saved_callee: Vec<PhysReg> = config
                .available_regs
                .iter()
                .filter(|r| used_regs_set.contains(&r.0))
                .copied()
                .collect();
            if !saved_callee.is_empty() {
                // Normalized occupancy union per saved register, from every
                // current holder's group-merged fat bounds read half-open,
                // `[def, last use)` -- Phase 2f's convention
                // (`segment_sets_overlap`): a value born where another dies
                // may take its register; an empty `[p, p)` still overlaps
                // any holder live strictly across `p`. A holder without
                // bounds blocks its register for the whole function.
                let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
                for (&vid, &reg) in &assignments {
                    if !saved_callee.contains(&reg) {
                        continue;
                    }
                    let span =
                        allocation_owner_bounds(vid, &merged_of, &iv_map).unwrap_or((0, u32::MAX));
                    occupied.entry(reg.0).or_default().push(span);
                }
                for spans in occupied.values_mut() {
                    spans.sort_unstable();
                    let source = std::mem::take(spans);
                    insert_segment_union(spans, &source);
                }
                // Phase 2f's ranking: the whole web's loop-weighted uses,
                // then the shorter envelope (fewer holes consumed, more
                // followers admitted), then the value id for determinism.
                let candidates: Vec<(u64, u32, u32, (u32, u32))> = phase2c_intervals
                    .iter()
                    .filter_map(|iv| {
                        let (start, end) =
                            allocation_owner_bounds(iv.value_id, &merged_of, &iv_map)?;
                        let own = use_count.get(&iv.value_id).copied().unwrap_or(0);
                        let group = coalesce_groups
                            .get(&iv.value_id)
                            .map_or(0, |members| summed_use_weight(members, &use_count));
                        Some((
                            own.max(group),
                            end.saturating_sub(start),
                            iv.value_id,
                            (start, end),
                        ))
                    })
                    .collect();
                for (vid, reg) in
                    fill_saved_callee_stretches(candidates, &mut occupied, &saved_callee)
                {
                    assignments.insert(vid, reg);
                }
            }
        }
    }
    propagate_coalesce_members(&mut assignments, &coalesce_member_of);
    if config.ra_config.debug_ra_phases {
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
    if !config.ra_config.no_load_hazard_refine && i686_scratch_pool(config) {
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
                &config.ra_config,
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
                    .filter(|&(_, &r)| r == reg)
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
                let mut alloc =
                    LinearScanAllocator::new_with_config(ranges, vec![reg], &config.ra_config);
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
    if !config.ra_config.no_eax_alloc && i686_scratch_pool(config) {
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
        // A coalesced leader may claim %eax only when every web member is
        // accumulator-first. Otherwise a non-first member inherits the home
        // and is clobbered by the consumer's early write.
        for (leader, members) in &coalesce_groups {
            let all_acc_first = members
                .iter()
                .all(|member| acc_first_uses.contains(member) && !non_acc_first.contains(member));
            if all_acc_first {
                acc_first_uses.insert(*leader);
            } else {
                acc_first_uses.remove(leader);
            }
        }

        let reg = PhysReg(6);
        let holders: Vec<(u32, u32)> = assignments
            .iter()
            .filter(|&(_, &r)| r == reg)
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
            let mut alloc =
                LinearScanAllocator::new_with_config(ranges, vec![reg], &config.ra_config);
            alloc.run();
            for (vid, r) in alloc.assignments {
                assignments.insert(vid, r);
                caller_used_regs_set.insert(r.0);
            }
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
        }
    }

    // AArch64-only: steal a callee-saved from a colder holder for a missed IV.
    // This is separate from the web-aware, globally priced x86 phase below.
    // Local exchanges inside the scan previously lost gzip.
    if !config.ra_config.no_loop_pin && arm_fp_pool && !all_phi_pairs.is_empty() {
        let k = config.ra_config.loop_pin;
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
                let cost = summed_use_weight(&evict, &use_count);
                if cost >= hot_count {
                    continue;
                }
                if best
                    .as_ref()
                    .is_none_or(|&(best_reg, _, c)| (cost, reg_id) < (c, best_reg))
                {
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
    // Phase 2g (i686/x86-64): globally priced hot-web steals.
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
    let i686_narrow_pool =
        i686_scratch_pool(config) && config.available_regs.iter().all(|r| r.0 <= 3);
    // Use the target and the actual post-clobber pool, not XMM availability:
    // -mgeneral-regs-only still needs GPR promotion. Empty/disabled pools and
    // reserved rbp never gain a register here. No caller-saved/APX home is
    // promoted to an ABI-preserved register.
    let x86_64_pool = crate::common::types::target_elf_machine() == 62
        && !config.available_regs.is_empty()
        && config.available_regs.iter().all(|r| (1..=6).contains(&r.0));
    if !config.ra_config.no_hot_web_steal && (i686_narrow_pool || x86_64_pool) {
        // Broaden allocation promotion, not phi coalescing, on the measured
        // x86-64 pool. Keep the established narrow-register i686 policy.
        let loop_carried = if x86_64_pool && sparse_call_state {
            hot_web::loop_carried_copy_dests(func, &liveness)
        } else {
            FxHashSet::default()
        };
        hot_web::steal_hot_loop_webs(
            &mut assignments,
            &hot_web::HotWebSteal {
                config,
                phi_pairs: &all_phi_pairs,
                loop_carried: &loop_carried,
                eligible: &eligible,
                iv_map: &iv_map,
                merged_of: &merged_of,
                coalesce_groups: &coalesce_groups,
                coalesce_member_of: &coalesce_member_of,
                use_count: &use_count,
            },
        );
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
    if !config.ra_config.no_iterated_hazard && i686_scratch_pool(config) {
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
                &config.ra_config,
            );
            if config.ra_config.debug_ra_intervals {
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
                    .filter(|&(_, &r)| r == reg)
                    .filter_map(|(&v, _)| iv_map.get(&v).copied())
                    .collect();
                if round == 0 && config.ra_config.debug_ra_intervals {
                    let holders_dbg: Vec<String> = assignments
                        .iter()
                        .filter(|&(_, &r)| r == reg)
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
                if config.ra_config.debug_ra_intervals {
                    #[expect(unused_mut)]
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
                let mut alloc =
                    LinearScanAllocator::new_with_config(ranges, vec![reg], &config.ra_config);
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
            if config.ra_config.debug_ra_intervals {
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
    if !config.ra_config.no_segment_fill && !used_regs_set.is_empty() {
        let owner_of = |v: u32| coalesce_member_of.get(&v).copied().unwrap_or(v);

        let mut owned_segments = owned_live_segments(&liveness, &coalesce_member_of);
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
                    occupied_by_reg
                        .entry(reg.0)
                        .or_default()
                        .extend(guard_internal_hole_boundaries(segments));
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
            let group = coalesce_groups
                .get(&value)
                .map_or(0, |members| summed_use_weight(members, &use_count));
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
        let fill_limit = config.ra_config.segment_fill_limit.filter(|_| {
            config
                .ra_config
                .segment_fill_func
                .as_deref()
                .is_none_or(|f| f == func.name)
        });
        for (_, _, value) in candidates {
            if fill_limit.is_some_and(|limit| added >= limit) {
                break;
            }
            let Some(candidate_segments) = owned_segments.get(&value) else {
                continue;
            };
            let Some(reg) =
                try_fill_segment_candidate(candidate_segments, &mut occupied_by_reg, &pool)
            else {
                continue;
            };
            if config.ra_config.debug_segment_fill {
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
            added += 1;
        }
        if added != 0 {
            propagate_coalesce_members(&mut assignments, &coalesce_member_of);
        }
        if config.ra_config.debug_segment_fill {
            eprintln!("[RA-SEGMENT-FILL] fn={} added={}", func.name, added);
        }
    }

    if config.ra_config.debug_ra {
        let mut v: Vec<(u32, u8)> = assignments.iter().map(|(k, r)| (*k, r.0)).collect();
        v.sort_unstable();
        eprintln!("[RA] fn={} FINAL={:?}", func.name, v);
        if config.ra_config.debug_ra_intervals {
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

    // Secondary latch copies of an already-represented phi dest (the if/else
    // latch `x = f(old); if (rare) x' = g(x); old = x | x'`: one copy per
    // arm).  Only the primary source was made home-less above, so the
    // sieve-class hazard (every source competing for one register, losers
    // spilling) cannot recur: a secondary keeps the home Phase 2 gave it and
    // the apply phase moves it onto the dest's register only when no foreign
    // holder conflicts, deleting that arm's copy.  Primaries go first so the
    // home-less sources always get their register.  Proofs are per pair and
    // web-aware (`source_home_survives_dest_redefs_in` dirties the home at a
    // sibling source's definition), so the arms' sources may share it.
    let phi_apply: Vec<PhiCoalesceCandidate> = if config.ra_config.no_phi_coalesce {
        Vec::new()
    } else {
        // Set membership keeps this linear in the pair count (a
        // `Vec::contains` filter was quadratic on large functions).
        let primary: FxHashSet<PhiCoalesceCandidate> = phi_coalesce.iter().copied().collect();
        let mut list = phi_coalesce.clone();
        list.extend(
            all_phi_pairs
                .iter()
                .filter(|c| !primary.contains(c))
                .copied(),
        );
        list
    };
    let mut applied_phi_coalesce = apply_phi_coalesce_assignments_with_config(
        func,
        &liveness,
        &iv_map,
        &phi_apply,
        &mut assignments,
        &config.available_regs,
        Some((&coalesce_member_of, &config.caller_saved_regs)),
        &config.ra_config,
    );

    let vector_values = if arm_fp_pool {
        collect_vector_values(func)
    } else if x86_fp_pool {
        let mut values = if !config.ra_config.no_reduction_vecreg {
            collect_x86_reduction_vector_values(func)
        } else {
            FxHashSet::default()
        };
        if !config.ra_config.no_map_vecreg {
            values.extend(collect_x86_map_broadcast_values(func));
            values.extend(collect_x86_map_intermediate_values(func));
        }
        values
    } else {
        FxHashSet::default()
    };
    let f64_value_set = if arm_fp_pool || (x86_fp_pool && !config.ra_config.no_fp_copy_web) {
        collect_f64_values(func, config.ra_config.fp_extract_homes)
    } else {
        FxHashSet::default()
    };

    let mut fp_web_member_of: FxHashMap<u32, u32> = FxHashMap::default();
    if !config.xmm_regs.is_empty() {
        // Destructive-form pre-allocation for SSE-128 vector chains: values
        // produced by a two-operand SSE intrinsic whose first operand dies
        // at that very instruction (ARX rounds, integer/FP chains routed
        // through emit_sse_binary_128), plus those dying operands.  They
        // are homed here in instruction order — a register recycles at the
        // dying operand, so the in-place emitter paths (`op %src, %dst`,
        // `pshufb %mask, %dst`, `pshufd $i, %src, %dst`) fire with zero
        // staging movdqas.  The f64 scan below then fills the remaining
        // registers for everything else (FP scalars, broadcasts, loads,
        // copy webs).
        let mut sse_chain_regs: FxHashSet<u8> = FxHashSet::default();
        let mut sse_chain_alloc: FxHashSet<u32> = FxHashSet::default();
        if x86_fp_pool {
            let sse_chain_values = collect_sse128_chain_values(func);
            if !sse_chain_values.is_empty() {
                let mut sse_chain_pool: Vec<PhysReg> = config
                    .xmm_regs
                    .iter()
                    .filter(|r| r.0 >= 21)
                    .copied()
                    .collect();
                sse_chain_pool.retain(|r| !assignments.values().any(|a| a.0 == r.0));
                if !sse_chain_pool.is_empty() {
                    let coalesced = allocate_vector_registers_destructive(
                        func,
                        &sse_chain_values,
                        &sse_chain_pool,
                        &|vid: u32| assignments.contains_key(&vid),
                        &liveness,
                    );
                    sse_chain_alloc = coalesced.iter().map(|&(v, _)| v).collect();
                    for (vid, reg) in coalesced {
                        sse_chain_regs.insert(reg.0);
                        assignments.insert(vid, reg);
                    }
                }
            }
        }
        // COLD-CALL XMM SPLIT: SysV x86-64 has no callee-saved XMM, so a
        // scalar FP value live across ANY call used to lose the pool and
        // round-trip the stack at every def and use -- including a loop
        // accumulator whose only crossing call is a never-taken slow path
        // (the inlined `sqrt` errno call: nbody-style loops ran ~2x the
        // instructions).  Values whose every crossing call is cold may take
        // a caller-saved XMM home; the save/restore pair is emitted around
        // those cold calls only (`caller_save_spans`, derived from the final
        // assignments below).  Scalars only: the save slot is 8 bytes.
        // x86-64 only: the i686 emitter does not implement the spans, and
        // a returns-twice call forbids caller-save homes outright.
        let cold_fp_spanning: FxHashSet<u32> = if x86_fp_pool
            && !is_32bit
            && !has_nonlocal_control
            && !config.ra_config.no_cold_call_split
        {
            let mut set =
                collect_cold_only_call_spanning_owners(&liveness, &iv_map, &coalesce_member_of);
            set.retain(|v| f64_value_set.contains(v) && !vector_values.contains(v));
            set
        } else {
            FxHashSet::default()
        };
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

        let mut f64_intervals: Vec<LiveInterval> = liveness
            .intervals
            .iter()
            .filter(|iv| non_gpr_values.contains(&iv.value_id))
            .filter(|iv| iv.end > iv.start)
            .filter(|iv| !assignments.contains_key(&iv.value_id))
            .filter(|iv| {
                !call_spanning.contains(&iv.value_id) || cold_fp_spanning.contains(&iv.value_id)
            })
            // The destructive-form pre-allocation above already homed the
            // SSE-128 chain values; never double-book them.
            .filter(|iv| !(x86_fp_pool && sse_chain_alloc.contains(&iv.value_id)))
            .filter(|iv| !(arm_fp_pool || x86_fp_pool) || real_use.contains(&iv.value_id))
            .filter(|iv| {
                vector_values.contains(&iv.value_id) || f64_value_set.contains(&iv.value_id)
            })
            .copied()
            .collect();

        // Follow-up #3: merge copy-connected scalar FP webs (combine → entry
        // copy → carry value → latch copy) into single leader intervals so
        // the linear scan homes the whole web in one XMM register and the
        // transport copies become same-register no-ops.  A group only
        // merges when at least two members carry real intervals (a lone
        // interval is left untouched); members are dropped from the scan
        // and later inherit the leader's register.
        // x86-only: the horizontal-add admission and direct-emit live in the
        // x86 SSE domain; the AArch64 NEON pool keeps its existing behavior.
        let fp_web_groups = if x86_fp_pool {
            fp_copy_web_groups(
                func,
                &f64_value_set,
                &real_use,
                &assignments,
                &liveness.segments,
            )
        } else {
            FxHashMap::default()
        };
        if !fp_web_groups.is_empty() {
            let member_intervals: FxHashMap<u32, (u32, u32)> = f64_intervals
                .iter()
                .map(|iv| (iv.value_id, (iv.start, iv.end)))
                .collect();
            let mut dropped: FxHashSet<u32> = FxHashSet::default();
            let mut leaders: Vec<LiveInterval> = Vec::new();
            for (leader, members) in &fp_web_groups {
                let mut start = u32::MAX;
                let mut end = 0u32;
                let mut counted = 0u32;
                for &m in members {
                    if let Some(&(s, e)) = member_intervals.get(&m) {
                        start = start.min(s);
                        end = end.max(e);
                        counted += 1;
                    }
                }
                if counted >= 2 && start < end {
                    leaders.push(LiveInterval {
                        value_id: *leader,
                        start,
                        end,
                    });
                    // Only members that actually carried a scan interval may
                    // inherit the leader's register: a member filtered out of
                    // the scan (call-spanning, zero-length, not in
                    // non_gpr_values) keeps its existing home and the copy
                    // stays a real move.  Register-homing a call-spanning
                    // value into a caller-saved pool register would let the
                    // call clobber it.
                    for &m in members {
                        if m != *leader && member_intervals.contains_key(&m) {
                            fp_web_member_of.insert(m, *leader);
                            dropped.insert(m);
                        }
                    }
                    // Drop the original leader envelope too: the merged
                    // leader interval is the only scan seed for the web.
                    dropped.insert(*leader);
                }
            }
            f64_intervals.retain(|iv| !dropped.contains(&iv.value_id));
            f64_intervals.extend(leaders);
        }

        if !f64_intervals.is_empty() {
            let (mut f64_ranges, f64_meta) = live_range::build_live_ranges_with_config_and_meta(
                &f64_intervals,
                &liveness.block_loop_depth,
                func,
                &config.ra_config,
                None,
            );
            // Slot-operand classification for the XMM pool, so a hoisted FP
            // constant whose every use folds its slot as a memory operand is
            // the scan's free victim instead of a temporary that would
            // round-trip the stack (`select_evict_victim`). Only the
            // classification is shared with the GPR scan; the loop-span
            // admission machinery of `mark_loop_spanning` is GPR-tuned.
            if x86_fp_pool {
                let mut fp_members_of: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
                for (&member, &leader) in &fp_web_member_of {
                    fp_members_of.entry(leader).or_default().push(member);
                }
                live_range::mark_slot_operand_only(
                    &mut f64_ranges,
                    func,
                    &fp_members_of,
                    &f64_meta.uses,
                );
            }
            // RA-05: hole-aware coverage for the XMM scan as well — FP phi
            // webs spanning mutually exclusive arms get the same treatment
            // as the GPR scan. Use the FP web map so member segments attach
            // to the merged leader, not to the GPR coalesce owner.
            attach_scan_segments(&mut f64_ranges, &liveness, &fp_web_member_of);
            let scan_pool: Vec<PhysReg> = if x86_fp_pool {
                config
                    .xmm_regs
                    .iter()
                    .filter(|r| !sse_chain_regs.contains(&r.0))
                    .copied()
                    .collect()
            } else {
                config.xmm_regs.clone()
            };
            let mut xmm_allocator =
                LinearScanAllocator::new_with_config(f64_ranges, scan_pool, &config.ra_config);
            xmm_allocator.free_slot_victims = x86_fp_pool;
            xmm_allocator.run();
            for (&vid, &reg) in &xmm_allocator.assignments {
                assignments.insert(vid, reg);
            }
            // Spread the leader's register over every merged web member so
            // the copies between them vanish.
            for (member, leader) in &fp_web_member_of {
                if let Some(&reg) = xmm_allocator.assignments.get(leader) {
                    assignments.insert(*member, reg);
                }
            }
        }

        if !config.ra_config.no_vecreg {
            let vec_candidates = collect_vecreg_candidates(func);
            if !vec_candidates.is_empty() {
                // The vector pool is the target's XMM family minus xmm2
                // (implicit scratch for a handful of intrinsic emitters)
                // minus everything earlier scans already claimed.  x86-64
                // therefore gets xmm3..xmm15; i686 and other targets shrink
                // with their own `xmm_regs`.
                let mut vec_pool: Vec<PhysReg> = config
                    .xmm_regs
                    .iter()
                    .filter(|r| r.0 >= 21)
                    .copied()
                    .collect();
                vec_pool.retain(|r| !assignments.values().any(|a| a.0 == r.0));
                if !vec_pool.is_empty() {
                    let vec_intervals =
                        synthetic_vec_intervals(func, &vec_candidates, &liveness.block_loop_depth);
                    let vec_intervals: Vec<LiveInterval> = vec_intervals
                        .into_iter()
                        .filter(|iv| !assignments.contains_key(&iv.value_id))
                        .filter(|iv| {
                            !half_open_range_contains_any_point(iv.start, iv.end, call_points)
                        })
                        .collect();
                    if !vec_intervals.is_empty() {
                        // The vecreg-ALLOCA scheme keeps its original
                        // LinearScanAllocator over the SYNTHETIC intervals:
                        // those values mirror 16-byte stack slots, and the
                        // synthetic interval construction (loop-region
                        // growth, split-layout merging) is the soundness
                        // contract for when a slot's register mirror may be
                        // recycled.  PR #455 replaced this with the raw
                        // linear-span destructive allocator and silently
                        // miscompiled user-level __m128i code (the
                        // simd_sse2_arith corpus); the destructive form is
                        // reserved for the SSE-128 CHAIN values above,
                        // which own no slots.
                        let vec_ranges = live_range::build_live_ranges_with_config(
                            &vec_intervals,
                            &liveness.block_loop_depth,
                            func,
                            &config.ra_config,
                        );
                        let mut vec_allocator = LinearScanAllocator::new_with_config(
                            vec_ranges,
                            vec_pool,
                            &config.ra_config,
                        );
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
            // d24–d31 (IDs 48..55) are caller-saved: a promoted value whose
            // live coverage spans a call must not take a reserved home. The
            // promotion pass only builds call-free loops, but the value's
            // range can still reach a call on exit/outer paths. Skipped
            // values fall through to the normal FP scan below.
            let owner = coalesce_member_of.get(&value.0).copied().unwrap_or(value.0);
            if call_spanning.contains(&owner) || call_spanning.contains(&value.0) {
                continue;
            }
            assignments.insert(value.0, PhysReg(48 + index as u8));
        }
    }

    if arm_fp_pool || !vector_values.is_empty() || !f64_value_set.is_empty() {
        // Segment coverage for the phi-move conflict test (built once;
        // no web map — webs only merge coverage, and the test must see
        // each third value's own pieces).
        let no_webs: FxHashMap<u32, u32> = FxHashMap::default();
        let fp_phi_seg_cov = owned_live_segments(&liveness, &no_webs);
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
            if fp_phi_move_conflicts(
                &fp_phi_seg_cov,
                &iv_map,
                &assignments,
                candidate.phi_dest,
                candidate.backedge_src,
                d.0,
            ) {
                continue;
            }
            // Sharing the dest home is a new live range for the source.
            // Caller-saved XMM/volatile NEON cannot inherit a home that
            // the source already needed to survive a call.
            let src_live = LiveInterval {
                start: src_iv.0,
                end: src_iv.1,
                value_id: candidate.backedge_src,
            };
            let dest_callee_fp = arm_fp_pool && (32..=38).contains(&d.0);
            if spans_any_call(&src_live, call_points) && !dest_callee_fp {
                continue;
            }
            assignments.insert(candidate.backedge_src, d);
            applied_phi_coalesce.push(*candidate);
        }
    }

    let mut caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
    if !config.caller_saved_regs.is_empty() && config.ra_config.caller_save_spanning {
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
                let mut span_allocator = LinearScanAllocator::new_with_config(
                    phase2b_ranges,
                    span_regs,
                    &config.ra_config,
                );
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

    // used_regs_set is the running "already-saved" pool for later phases
    // (segment fill, spanning). The prologue save-set is rebuilt AFTER
    // repair and CCC_RA_DROP from the homes that actually survive.

    if !config.ra_config.no_save_set_compact {
        let moves = compact_callee_saved_homes(&mut assignments, &iv_map, &config.available_regs);
        if config.ra_config.debug_ra && !moves.is_empty() {
            eprintln!("[RA-COMPACT] fn={} folded {:?}", func.name, moves);
        }
    }

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
    // stack slots — always sound). Evictions only remove homes, never add, so
    // no new overlap can appear; iterate to a fixpoint so the compiler stays
    // total even on adversarial inputs (a single pass plus a hard assert
    // would panic the whole compilation if enumeration ever disagreed with
    // the victim selection). Every round with conflicts evicts at least one
    // class, and classes are finite, so the loop terminates; the round cap is
    // a fail-closed backstop that evicts every remaining conflicting class.
    {
        let mut parent: FxHashMap<u32, u32> = assignments.keys().map(|&v| (v, v)).collect();
        for (&member, &leader) in &coalesce_member_of {
            unite_map(&mut parent, member, leader);
        }
        // Only actually-applied destructive updates share a home at this
        // point; rejected candidates must not bless overlaps.
        for pair in &applied_phi_coalesce {
            unite_map(&mut parent, pair.phi_dest, pair.backedge_src);
        }
        for (&member, &leader) in &fp_web_member_of {
            unite_map(&mut parent, member, leader);
        }
        flatten_allocation_classes(&mut parent);
        let max_rounds = parent.len().saturating_add(2);
        let mut round = 0u32;
        loop {
            let rep = find_overlapping_classes(&liveness, &assignments, &parent);
            if rep.is_empty() {
                break;
            }
            round += 1;
            if config.ra_config.debug_ra_repair {
                eprintln!(
                    "[RA-REPAIR] fn={} round={} scanned assignments={} classes={} overlaps={}",
                    func.name,
                    round,
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
            // Evict the colder class of each conflicting pair (fewer total
            // uses; ties break to the LATER value id so the eviction is
            // deterministic).
            let mut class_weights: FxHashMap<u32, u64> = FxHashMap::default();
            for &value in assignments.keys() {
                let class = parent.get(&value).copied().unwrap_or(value);
                let weight = use_count.get(&value).copied().unwrap_or(0);
                let total = class_weights.entry(class).or_insert(0);
                *total = total.saturating_add(weight);
            }
            let mut evict_classes: FxHashSet<u32> = FxHashSet::default();
            if round > max_rounds as u32 {
                // Unreachable in practice (each round removes a class); if it
                // ever triggers, evict every conflicting class fail-closed.
                for &(_reg, a, b, _s, _e) in &rep {
                    evict_classes.insert(a);
                    evict_classes.insert(b);
                }
            } else {
                for &(_reg, a, b, _s, _e) in &rep {
                    if evict_classes.contains(&a) || evict_classes.contains(&b) {
                        continue;
                    }
                    let wa = class_weights.get(&a).copied().unwrap_or(0);
                    let wb = class_weights.get(&b).copied().unwrap_or(0);
                    let loser = if wa != wb {
                        if wa < wb { a } else { b }
                    } else if a < b {
                        b
                    } else {
                        a
                    };
                    evict_classes.insert(loser);
                }
            }
            let mut evicted: Vec<u32> = Vec::new();
            for (&v, &r) in parent.iter() {
                if evict_classes.contains(&r) && assignments.remove(&v).is_some() {
                    evicted.push(v);
                }
            }
            if config.ra_config.debug_ra_repair {
                eprintln!(
                    "[RA-REPAIR] fn={} round={} evicted {:?} (classes {:?})",
                    func.name, round, evicted, evict_classes
                );
            }
        }
        debug_assert!(
            find_overlapping_classes(&liveness, &assignments, &parent).is_empty(),
            "RA repair left overlapping classes in {}",
            func.name
        );
    }

    // IPA-RA fill (x86-64 `ipa_ra`): after every phase and the repair (homes
    // are final), before the overlap verifier, which therefore audits it too.
    // Running last means it never competes with Phase 2: a first version ran
    // before Phase 2 and seeded its waves, and on hot_web_loop the cold
    // values it homed over [5,60] took %r8/%r9 from the loop temporaries,
    // costing a hot-path copy (+3.3% Ir) to save cold-path loads. Here a
    // tenant is only ever displaced into another free caller-saved register
    // it is allowed in (Phase 2's per-wave pool rules, fold-base registers
    // for deferred-fold bases), never to memory.
    let mut ipa_used: Vec<PhysReg> = Vec::new();
    if !config.ipa_regs.is_empty() {
        let restricted = |v: u32| !ordered_param_homes && param_restricted.contains(&v);
        let mut fixed: FxHashSet<u32> = FxHashSet::default();
        for (&m, &l) in coalesce_member_of.iter().chain(fp_web_member_of.iter()) {
            fixed.insert(m);
            fixed.insert(l);
        }
        for (&leader, members) in &coalesce_groups {
            fixed.insert(leader);
            fixed.extend(members.iter().copied());
        }
        for pair in &applied_phi_coalesce {
            fixed.insert(pair.phi_dest);
            fixed.insert(pair.backedge_src);
        }
        for inst in func.blocks.iter().flat_map(|b| b.instructions.iter()) {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                fixed.insert(dest.0);
                for (op, _) in incoming {
                    if let Operand::Value(v) = op {
                        fixed.insert(v.0);
                    }
                }
            }
        }
        let candidates: Vec<u32> = {
            let mut c: Vec<u32> = call_spanning
                .iter()
                .copied()
                .filter(|v| {
                    eligible.contains(v)
                        && !non_gpr_values.contains(v)
                        && !fixed.contains(v)
                        && !restricted(*v)
                        && !assignments.contains_key(v)
                })
                .collect();
            c.sort_unstable();
            c
        };
        let scratch_demand: Vec<(u32, u32)> = {
            let mut d: Vec<(u32, u32)> = iv_map
                .iter()
                .filter(|(v, _)| {
                    eligible.contains(v)
                        && !assignments.contains_key(v)
                        && !call_spanning.contains(v)
                        && !non_gpr_values.contains(v)
                })
                .map(|(_, &iv)| iv)
                .collect();
            d.sort_unstable();
            d
        };
        let reloc_ok = |v: u32, r: PhysReg| {
            !fixed.contains(&v)
                && !call_spanning.contains(&v)
                && !restricted(v)
                && !non_gpr_values.contains(&v)
                && !(later_arg_values.contains(&v) && config.call_arg_regs.contains(&r))
                && !(indirect_arg_values.contains(&v) && config.indirect_target_regs.contains(&r))
                && (!fold_use_transfer.contains_key(&v) || config.fold_base_regs.contains(&r))
        };
        ipa_used = ipa_fill_preserved_regs(
            &mut assignments,
            &candidates,
            &IpaFill {
                ipa_regs: &config.ipa_regs,
                callee_pool: &config.available_regs,
                reloc_pool: &config.caller_saved_regs,
                iv_map: &iv_map,
                cost: &|v| use_count.get(&v).copied().unwrap_or(0),
                reloc_ok: &reloc_ok,
                rename_blocked: &restricted,
                scratch_demand: &scratch_demand,
                trace: config.ra_config.debug_ra,
            },
        );
        if config.ra_config.debug_ra && !ipa_used.is_empty() {
            eprintln!("[RA-IPA] fn={} used {:?}", func.name, ipa_used);
        }
    }

    if config.ra_config.verify_regalloc {
        verify_no_overlap(
            &liveness,
            &assignments,
            &coalesce_member_of,
            &applied_phi_coalesce,
        );
    }

    // Session-28 debug: per-value home census (register vs slot) for one
    // function, to analyze spill/slot-traffic decisions.
    if config.ra_config.legacy_debug_ra {
        let filter = &config.ra_config.legacy_debug_ra_func;
        if filter.is_empty() || func.name.contains(filter.as_str()) {
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

    if let Some(filter) = &config.ra_config.trace_allocstats_filter {
        if filter.is_empty() || filter == "*" || func.name.contains(filter.as_str()) {
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

    if let Some(filter) = &config.ra_config.ra_explain {
        if filter.is_empty() || filter == "*" || func.name.contains(filter.as_str()) {
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
            // Assignment dump (CCC_RA_EXPLAIN_HOMES=1): every register-homed
            // value with its physical register, fat interval and hole-aware
            // segments. This is the primary tool for diagnosing "two values
            // share a register while both are live" classes of miscompile:
            // the segments printed here are exactly what the scan and the
            // verifier reason about, so a read the IR does not show (folded
            // SIB index, replayed cmp operand) is visible as a segment gap.
            if config.ra_config.ra_explain_homes {
                let mut homes: Vec<(u32, PhysReg)> =
                    assignments.iter().map(|(&v, &r)| (v, r)).collect();
                homes.sort_unstable_by_key(|&(v, _)| v);
                for (v, reg) in homes {
                    let fat = iv_map.get(&v).copied();
                    let mut segs: Vec<(u32, u32)> = liveness
                        .segments
                        .iter()
                        .filter(|s| s.value_id == v)
                        .map(|s| (s.start, s.end))
                        .collect();
                    segs.sort_unstable();
                    eprintln!(
                        "[RA-EXPLAIN] home v{} reg={} fat={:?} segments={:?}",
                        v, reg.0, fat, segs
                    );
                }
            }
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

    // Bisect aid: CCC_RA_DROP=<vid,...> (scoped by CCC_RA_DROP_FUNC=<name>)
    // demotes the listed values to stack slots after every allocation
    // authority has run. Dropping a home is always sound, so a delta-debug
    // over the home list isolates the one value whose register home the
    // emitted code violates (used to localize the preboot-ZSTD failures).
    if let Some(list) = &config.ra_config.ra_drop {
        let scoped = config
            .ra_config
            .ra_drop_func
            .as_deref()
            .is_some_and(|f| f != func.name);
        if !scoped {
            for tok in list.split(',') {
                let tok = tok.trim();
                let tok = tok.strip_prefix('v').unwrap_or(tok);
                if let Ok(v) = tok.parse::<u32>() {
                    if assignments.remove(&v).is_some() && config.ra_config.debug_ra {
                        eprintln!("[RA-DROP] fn={} v{} demoted to slot", func.name, v);
                    }
                }
            }
        }
    }

    let final_reg_ids: FxHashSet<u8> = assignments.values().map(|reg| reg.0).collect();
    let mut used_regs: Vec<PhysReg> = config
        .available_regs
        .iter()
        .copied()
        .filter(|reg| final_reg_ids.contains(&reg.0))
        .collect();
    used_regs.sort_unstable_by_key(|reg| reg.0);
    used_regs.dedup();
    caller_save_spans.retain(|reg, spans| final_reg_ids.contains(reg) && !spans.is_empty());
    add_cold_call_xmm_saves(
        &assignments,
        &liveness,
        &iv_map,
        x86_fp_pool && !is_32bit,
        &mut caller_save_spans,
    );

    let mut accumulator_assignments = analyze_accumulator_assignments_with_config(
        func,
        config.accumulator_policy,
        &config.ra_config,
    );
    // A physical assignment is the durable home and always wins. Publishing
    // both locations made downstream behavior depend on insertion order.
    accumulator_assignments.retain(|a| !assignments.contains_key(&a.value_id));
    verify_accumulator_assignments(func, &accumulator_assignments);

    // Collapse the verifier's blessed same-value classes into one union-find
    // and publish each class under its minimum member id. Only classes the
    // overlap repair already unions may be published: regular copy webs,
    // actually-applied phi destructive updates, and FP webs. Rejected phi
    // candidates never bless overlaps and must not alias here either.
    let mut chain_parent: FxHashMap<u32, u32> = assignments.keys().map(|&v| (v, v)).collect();
    for (&member, &leader) in &coalesce_member_of {
        unite_map(&mut chain_parent, member, leader);
    }
    for pair in &applied_phi_coalesce {
        unite_map(&mut chain_parent, pair.phi_dest, pair.backedge_src);
    }
    for (&member, &leader) in &fp_web_member_of {
        unite_map(&mut chain_parent, member, leader);
    }
    let mut chain_roots: FxHashMap<u32, u32> = FxHashMap::default();
    for v in chain_parent.keys() {
        let mut r = *v;
        while chain_parent[&r] != r {
            r = chain_parent[&r];
        }
        chain_roots.insert(*v, r);
    }
    let mut class_min: FxHashMap<u32, u32> = FxHashMap::default();
    for (&member, &root) in &chain_roots {
        let entry = class_min.entry(root).or_insert(member);
        *entry = (*entry).min(member);
    }
    let mut phi_chain: FxHashMap<u32, u32> = FxHashMap::default();
    for (&member, &root) in &chain_roots {
        let rep = class_min[&root].min(root);
        if member != rep {
            phi_chain.insert(member, rep);
        }
    }

    if config.ra_config.debug_ra_phases {
        // Value -> register after EVERY phase (steal, hazard refinement,
        // segment fill): which value bought a callee-saved register, which
        // share one. `[RA-FINAL]` above is the main scan's state only.
        let mut ids: Vec<u32> = assignments.keys().copied().collect();
        ids.sort_unstable();
        let homes: Vec<String> = ids
            .iter()
            .map(|v| format!("v{v}=r{}", assignments[v].0))
            .collect();
        eprintln!("[RA-HOMES] fn={} {}", func.name, homes.join(" "));
    }
    RegAllocResult {
        ipa_used,
        eligible,
        assignments,
        accumulator_assignments,
        used_regs,
        caller_save_spans,
        liveness: Some(liveness),
        phi_chain,
        fold_deferred: fold_use_transfer,
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
    flatten_allocation_classes(&mut parent);
    let overlaps = find_overlapping_classes(liveness, assignments, &parent);
    assert!(
        overlaps.is_empty(),
        "register-allocation overlap: {overlaps:?}"
    );
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
fn collect_vecreg_candidates(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp;

    let mut allocas: FxHashSet<u32> = FxHashSet::default();
    let mut volatile_allocas: FxHashSet<u32> = FxHashSet::default();
    let mut over_align_allocas: FxHashSet<u32> = FxHashSet::default();
    let mut wrong_size_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                align,
                size,
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
                // Legacy slot promotion is a 16-byte SSE register mirror.
                if *size != 16 {
                    wrong_size_allocas.insert(dest.0);
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
        // The destructive FP binary ops (add/sub/mul/div/min/max x ps/pd/sd/ss)
        // take exactly two 16-byte vector operands and coalesce with the first.
        // One shared predicate, not a fourth hand-maintained copy of the list:
        // `divps/divpd/minps/minpd/maxps/maxpd` were missing from the copies
        // here and in `copy_coalescing`, which kept both operands in stack
        // slots (20 instructions against 14 for the identical `addpd` kernel).
        if op.is_sse_fp_128_destructive_binary() {
            return Some(2);
        }
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
            | O::Pinsrq128
            | O::VecRotlI32x4
            | O::VecShufdI32x4
            // BB-SLP packed lane shifts: one vector operand plus a
            // constant immediate (the immediate never names a Value).
            | O::VecShlI16x16
            | O::VecShlI16x8
            | O::VecLShrI16x16
            | O::VecLShrI16x8
            | O::VecAShrI16x16
            | O::VecAShrI16x8
            | O::VecShlI32x8
            | O::VecShlI32x4
            | O::VecLShrI32x8
            | O::VecLShrI32x4
            | O::VecAShrI32x8
            | O::VecAShrI32x4
            | O::VecShlI64x4
            | O::VecShlI64x2
            | O::VecLShrI64x4
            | O::VecLShrI64x2
            // Horizontal counting exit: one vector operand, scalar result.
            | O::VecHorizontalAddI64x4 => Some(1),
            // Byte-predicate counting binaries: two vector operands each
            // (`vpsadbw a, b`, `vpaddq a, b`).
            | O::VecSadbwU8x32
            | O::VecAddI64x4 => Some(2),
            // Adler-32 epic binaries: `vpmaddubsw`/`vpmaddwd` each read two
            // vector operands.  The const-table producer reads none (its
            // 32 args are all constants).
            | O::VecMaddubsU8x32
            | O::VecMaddwdI16x16 => Some(2),
            O::VecConstI8x32 => Some(0),

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

            // Packed compares: lhs, rhs are vector args; the trailing
            // operand is the predicate CONSTANT (never a Value, but keep
            // the count at 2 so a malformed IR cannot smuggle a vector
            // read past the audit).
            O::VecCmpF32x8
            | O::VecCmpF32x4
            | O::VecCmpF64x4
            | O::VecCmpF64x2
            | O::VecCmpI32x8
            | O::VecCmpI32x4
            | O::VecCmpI64x4
            | O::VecCmpI64x2
         | O::VecCmpI8x32
            | O::VecCmpI8x16
            | O::VecMinU8x32
            | O::VecMinU8x16
            | O::VecMaxU8x32
            | O::VecMaxU8x16
            // Word-lane compares (OP-05g): same [a, b, imm] shape as the
            // dword/byte forms — two vector reads plus a constant
            // predicate.
            | O::VecCmpI16x16
            | O::VecCmpI16x8 => Some(2),
            // ARX lane family: rotate/lane-shuffle read one vector arg
            // (the amount/imm trail as immediates), the byte shuffle reads
            // two (data + mask), the pack's four inputs are scalars, and
            // the lane extract reads one vector arg.
            O::VecShufbI32x4 => Some(2),
            O::VecPackI32x4 => Some(0),
            O::VecExtractLaneI32x4 => Some(1),

            // Lane-mask selects: [false, true, mask] — all three are vector
            // reads resolved through the register cache by the AVX/SSE
            // blendv emitters.
            O::VecBlendvF32x8
            | O::VecBlendvF32x4
            | O::VecBlendvF64x4
            | O::VecBlendvF64x2
            | O::VecFmaF64x2
            | O::VecFnmaF64x2
            | O::VecFmaF32x4
            | O::VecFnmaF32x4
            | O::VecFmsF64x2
            | O::VecFnmsF64x2
            | O::VecFmsF32x4
            | O::VecFnmsF32x4
            | O::VecBlendvI32x8
            | O::VecBlendvI32x4
            | O::VecBlendvI8x32
            | O::VecBlendvI8x16
            | O::VecBlendvI16x16
            | O::VecBlendvI16x8 => Some(3),

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
                | O::MulAddF64x4HoistedSIB
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
                | O::VecLoadI32x4Pair
                | O::VecPackI32x4Pair
                | O::VecLoadI8x32
                | O::VecLoadI16x16
                | O::VecLoadF32x4
                | O::VecLoadF32x8
                | O::VecAddF64x2
                | O::VecAddF64x4
                | O::VecFmaF64x2
                | O::VecFnmaF64x2
                | O::VecFmaF32x4
                | O::VecFnmaF32x4
                | O::VecFmsF64x2
                | O::VecFnmsF64x2
                | O::VecFmsF32x4
                | O::VecFnmsF32x4
                | O::VecAddI32x4
                | O::VecAddI32x8
                | O::VecAddI8x32
                | O::VecAddF32x4
                | O::VecAddF32x8
                | O::VecMulF64x2
                | O::VecMulF64x4
                | O::VecMulF32x4
                | O::VecMulF32x8
                | O::VecMinI32x8
                | O::VecMaxI32x8
                | O::VecSubI8x32
                | O::VecCmpI8x32
                | O::VecMinU8x32
                | O::VecMaxU8x32
                | O::VecBlendvI8x32
                | O::VecAddI8x16
                | O::VecSubI8x16
                | O::VecCmpI8x16
                | O::VecMinU8x16
                | O::VecMaxU8x16
                | O::VecBlendvI8x16
                | O::VecMinI8x32
                | O::VecMaxI8x32
                | O::VecBroadcastI8x32
                | O::VecBroadcastI8x16
                | O::VecAddI16x16
                | O::VecSubI16x16
                | O::VecMulI16x16
                | O::VecCmpI16x16
                | O::VecMinI16x16
                | O::VecMaxI16x16
                | O::VecMinU16x16
                | O::VecMaxU16x16
                | O::VecBlendvI16x16
                | O::VecAddI16x8
                | O::VecSubI16x8
                | O::VecMulI16x8
                | O::VecCmpI16x8
                | O::VecMinI16x8
                | O::VecMaxI16x8
                | O::VecBlendvI16x8
                | O::VecBroadcastI16x16
                | O::VecBroadcastI16x8
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
                    } else {
                        // Unknown dest_ptr writer: memory updates without the
                        // promoted register mirror. Poison even if another
                        // known producer also wrote this slot.
                        bad_use.insert(d.0);
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
            || wrong_size_allocas.contains(&v)
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
    _block_loop_depth: &[u32],
) -> Vec<LiveInterval> {
    use std::collections::VecDeque;

    super::liveness::validate_program_point_capacity(func);
    let block_count = func.blocks.len();
    if block_count == 0 || candidates.is_empty() {
        return Vec::new();
    }

    let mut block_ranges: Vec<(u32, u32)> = Vec::with_capacity(block_count);
    let mut block_uses: Vec<FxHashSet<u32>> =
        (0..block_count).map(|_| FxHashSet::default()).collect();
    let mut block_defs: Vec<FxHashSet<u32>> =
        (0..block_count).map(|_| FxHashSet::default()).collect();
    let mut bounds: FxHashMap<u32, (u32, u32)> = FxHashMap::default();
    let mut point = 0u32;

    for (block_index, block) in func.blocks.iter().enumerate() {
        let block_start = point;
        for inst in &block.instructions {
            let end = point + 1;
            let mut read_values: FxHashSet<u32> = FxHashSet::default();
            let mut written_value: Option<u32> = None;
            match inst {
                Instruction::Intrinsic { dest_ptr, args, .. } => {
                    for arg in args {
                        if let Operand::Value(value) = arg {
                            if candidates.contains(&value.0) {
                                read_values.insert(value.0);
                            }
                        }
                    }
                    if let Some(dest) = dest_ptr {
                        if candidates.contains(&dest.0) {
                            written_value = Some(dest.0);
                        }
                    }
                }
                _ => {
                    for_each_operand_in_instruction(inst, |operand| {
                        if let Operand::Value(value) = operand {
                            if candidates.contains(&value.0) {
                                read_values.insert(value.0);
                            }
                        }
                    });
                    for_each_value_use_in_instruction(inst, |value| {
                        if candidates.contains(&value.0) {
                            read_values.insert(value.0);
                        }
                    });
                }
            }
            for value in read_values {
                if !block_defs[block_index].contains(&value) {
                    block_uses[block_index].insert(value);
                }
                bounds
                    .entry(value)
                    .and_modify(|range| {
                        range.0 = range.0.min(point);
                        range.1 = range.1.max(end);
                    })
                    .or_insert((point, end));
            }
            if let Some(value) = written_value {
                block_defs[block_index].insert(value);
                bounds
                    .entry(value)
                    .and_modify(|range| {
                        range.0 = range.0.min(point);
                        range.1 = range.1.max(end);
                    })
                    .or_insert((point, end));
            }
            point = end;
        }
        let terminator_end = point + 1;
        for_each_operand_in_terminator(&block.terminator, |operand| {
            let Operand::Value(value) = operand else {
                return;
            };
            if !candidates.contains(&value.0) {
                return;
            }
            if !block_defs[block_index].contains(&value.0) {
                block_uses[block_index].insert(value.0);
            }
            bounds
                .entry(value.0)
                .and_modify(|range| {
                    range.0 = range.0.min(point);
                    range.1 = range.1.max(terminator_end);
                })
                .or_insert((point, terminator_end));
        });
        point = terminator_end;
        block_ranges.push((block_start, point));
    }

    let labels = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &labels);
    let mut live_in: Vec<FxHashSet<u32>> = (0..block_count).map(|_| FxHashSet::default()).collect();
    let mut live_out: Vec<FxHashSet<u32>> =
        (0..block_count).map(|_| FxHashSet::default()).collect();
    let mut work: VecDeque<usize> = (0..block_count).rev().collect();
    let mut queued = vec![true; block_count];
    while let Some(block_index) = work.pop_front() {
        queued[block_index] = false;
        let mut new_out: FxHashSet<u32> = FxHashSet::default();
        for &successor in succs.row(block_index) {
            let successor = successor as usize;
            new_out.extend(live_in[successor].iter().copied());
        }
        let mut new_in = block_uses[block_index].clone();
        new_in.extend(
            new_out
                .iter()
                .copied()
                .filter(|value| !block_defs[block_index].contains(value)),
        );
        live_out[block_index] = new_out;
        if new_in != live_in[block_index] {
            live_in[block_index] = new_in;
            for &predecessor in preds.row(block_index) {
                let predecessor = predecessor as usize;
                if predecessor < block_count && !queued[predecessor] {
                    queued[predecessor] = true;
                    work.push_back(predecessor);
                }
            }
        }
    }

    for block_index in 0..block_count {
        let (block_start, block_end) = block_ranges[block_index];
        for &value in &live_in[block_index] {
            bounds
                .entry(value)
                .and_modify(|range| {
                    range.0 = range.0.min(block_start);
                })
                .or_insert((block_start, block_start));
        }
        for &value in &live_out[block_index] {
            bounds
                .entry(value)
                .and_modify(|range| {
                    range.1 = range.1.max(block_end);
                })
                .or_insert((block_start, block_end));
        }
    }

    let mut result: Vec<LiveInterval> = bounds
        .into_iter()
        .filter_map(|(value_id, (start, end))| {
            (start < end).then_some(LiveInterval {
                value_id,
                start,
                end,
            })
        })
        .collect();
    result.sort_unstable_by_key(|interval| (interval.start, interval.end, interval.value_id));
    result
}

/// Chain-family predicate shared by the SSE-128 collector and the
/// destructive allocator's handoff detector: two-operand 128-bit integer/FP
/// SIMD intrinsics (plus rotate/shuffle) whose destination may share its
/// first operand's home when that operand dies at the defining instruction.
fn is_fma3_op(op: &IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::VecFmaF64x2
            | O::VecFnmaF64x2
            | O::VecFmsF64x2
            | O::VecFnmsF64x2
            | O::VecFmaF32x4
            | O::VecFnmaF32x4
            | O::VecFmsF32x4
            | O::VecFnmsF32x4
            | O::VecMaddF32x8
            | O::VecMaddF32x8Signed(..)
            | O::VecMaddF64x4
            | O::VecMaddF64x4Signed(..)
    )
}

fn is_sse128_chain_op(op: &IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::VecAddI32x4
            | O::VecSubI32x4
            | O::VecMulI32x4
            | O::VecAndI32x4
            | O::VecOrI32x4
            | O::VecXorI32x4
            | O::VecAddI64x2
            | O::VecSubI64x2
            | O::VecAddI8x16
            | O::VecSubI8x16
            | O::VecMinU8x16
            | O::VecMaxU8x16
            | O::VecAddF32x4
            | O::VecSubF32x4
            | O::VecMulF32x4
            | O::VecDivF32x4
            | O::VecXorF32x4
            | O::VecMinF32x4
            | O::VecMaxF32x4
            | O::VecAddF64x2
            | O::VecSubF64x2
            | O::VecMulF64x2
            | O::VecDivF64x2
            | O::VecFmaF64x2
            | O::VecFnmaF64x2
            | O::VecFmaF32x4
            | O::VecFnmaF32x4
            | O::VecFmsF64x2
            | O::VecFnmsF64x2
            | O::VecFmsF32x4
            | O::VecFnmsF32x4
            // 256-bit FMA3 (`vecfma` contraction): destructive on x86 like
            // every FMA3 form.  `emit_avx_map_fma_inner` re-encodes the
            // 132/231 family for whichever operand the destination home
            // already holds, so a first-operand handoff turns the staged
            // `vmovaps a, d; vfmadd132ps b, c, d` pair into one instruction.
            | O::VecMaddF32x8
            | O::VecMaddF32x8Signed(..)
            | O::VecMaddF64x4
            | O::VecMaddF64x4Signed(..)
            | O::VecXorF64x2
            | O::VecMinF64x2
            | O::VecMaxF64x2
            | O::VecRotlI32x4
            | O::VecShufdI32x4
            | O::VecShufbI32x4
            // BB-SLP 128-bit families: the I16x8 mul/bitwise chains and
            // the I64x2/I8x16 bitwise chains use the same two-operand
            // emitters (emit_sse_binary_128), so the destructive chain
            // homing applies unchanged (xmm-dest + first-operand handoff).
            | O::VecMulI16x8
            | O::VecAndI16x8
            | O::VecOrI16x8
            | O::VecXorI16x8
            | O::VecAndI8x16
            | O::VecOrI8x16
            | O::VecXorI8x16
            | O::VecAndI64x2
            | O::VecOrI64x2
            | O::VecXorI64x2
            // GNU-vector 128-bit integer shapes that were missing from the
            // chain family and therefore never register-homed (every
            // `v8s + v8s` paid two prologue spills and four reloads):
            // word add/sub/min/max (emit_sse_binary_128), the four
            // integer lane compares (`emit_int_cmp*`: the all-homed VEX
            // path writes the destination last, so a first-operand
            // handoff is sound) and the VEX.128 variable shifts
            // (`emit_vex_binary_128`, three-operand).
            | O::VecAddI16x8
            | O::VecSubI16x8
            | O::VecMinI16x8
            | O::VecMaxI16x8
            | O::VecCmpI32x4
            | O::VecCmpI64x2
            | O::VecCmpI16x8
            | O::VecCmpI8x16
            | O::VecShlvI32x4
            | O::VecLShrvI32x4
            | O::VecAShrvI32x4
            | O::VecShlvI64x2
            | O::VecLShrvI64x2
    )
}

/// The GNU-vector scalar broadcasts of every family and width.
fn is_vector_broadcast_op(op: &IntrinsicOp) -> bool {
    use IntrinsicOp as O;
    matches!(
        op,
        O::VecBroadcastF32x8
            | O::VecBroadcastF64x4
            | O::VecBroadcastI32x8
            | O::VecBroadcastI64x4
            | O::VecBroadcastI16x16
            | O::VecBroadcastI8x32
            | O::VecBroadcastF32x4
            | O::VecBroadcastF64x2
            | O::VecBroadcastI32x4
            | O::VecBroadcastI64x2
            | O::VecBroadcastI16x8
            | O::VecBroadcastI8x16
    )
}

fn collect_sse128_chain_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut out: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest: Some(d),
                args,
                op,
                ..
            } = inst
            {
                // A vector broadcast's operand is a SCALAR (GP or XMM
                // scalar), never a chain operand: home only the result.
                // The emitters commit a homed destination through
                // `avx_store_dest` / `sse_store_dest` (one register copy
                // the peephole retargets away); unhomed, a broadcast that
                // feeds a terminator or a multi-use web paid a slot round
                // trip (`(v32i){} + x` after the zero fold is a bare
                // broadcast returned directly).
                if is_vector_broadcast_op(op) {
                    out.insert(d.0);
                    continue;
                }
                if is_sse128_chain_op(op) && !args.is_empty() {
                    if let Operand::Value(a0) = &args[0] {
                        let same_as_a1 = matches!(&args[1.min(args.len() - 1)], Operand::Value(v) if v.0 == a0.0);
                        // A squared multiplicand (`a*a + c`) is fine for
                        // FMA3: the 132 form reads both sources before the
                        // write, so `d == a == b` is exact.
                        if !same_as_a1 || is_fma3_op(op) {
                            // Home the chain DEST and its first operand
                            // unconditionally: each gets its own register
                            // whenever one is free for its live coverage
                            // (hole-aware segments) -- overlapping
                            // coverage never shares, so adding candidates
                            // is always sound.  A first operand whose last
                            // read IS this instruction additionally earns
                            // a proven handoff edge in the pass below,
                            // which recycles its register into the
                            // destination (the in-place form).  Restricting
                            // the
                            // collection to dying operands (PR #455's
                            // original form) left loop-header role values
                            // -- alive to a loop-exit materialisation --
                            // unhomed, and the block-edge phi copy then
                            // lowered to a per-iteration slot store+reload
                            // pair (measured: 26 of 74 instructions in the
                            // ChaCha double round were that roundtrip).
                            out.insert(d.0);
                            out.insert(a0.0);
                            // The second operand when it is a value
                            // (pshufb's loop-invariant MASK, a second
                            // stream): homed with its own register for its
                            // span, so a loop body reads it from the
                            // register instead of reloading its slot at
                            // every use (measured: 4 mask reloads per
                            // double round).  Const operands (rotate
                            // amounts, compare predicates) stay out.
                            if let Some(Operand::Value(a1)) = args.get(1) {
                                out.insert(a1.0);
                            }
                        }
                    }
                }
            }
        }
    }
    // Backward copy propagation of chain homes: the ARX vectorizer lowers
    // a loop-header phi's ENTRY edge to `Copy { dest: entry_val, src:
    // preheader_pack }`, and the same shape appears wherever a chain
    // value is renamed.  With only the chain dest homed, the copy still
    // round-trips its source through the slot (store in the preheader,
    // reload at the edge — the chacha20 0xd0-frame staging dance).  A
    // Copy source whose dest is a chain candidate earns a home too: the
    // destructive allocator below still runs its interference check over
    // every candidate's hole-aware coverage, so an overlapping value is
    // never displaced — admission is unconditionally sound.  Iterate to a
    // fixpoint so copy chains propagate fully (bounded by the copy count;
    // each round either inserts a new id or stops).
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src } = inst {
                    if out.contains(&dest.0) {
                        if let Operand::Value(s) = src {
                            if out.insert(s.0) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Env-gated trace for the destructive SSE-128 scan: prints candidates,
/// coverage, handoff edges, and assignments to stderr.  Zero cost when off;
/// the REACT loop for chain-coloring quality work.
fn sse_chain_debug_enabled() -> bool {
    std::env::var_os("LCCC_DEBUG_SSE_CHAIN").is_some()
}

/// Segment-precise third-value conflict test for FP phi-pair propagation.
///
/// The move shares `d_reg` between `phi_dest` and `backedge_src` (unioned
/// via `applied_phi_coalesce`, so the pair itself never conflicts).  It is
/// blocked only by a THIRD value homed in `d_reg` whose live coverage
/// overlaps the source's.  Coverage is hole-aware segments where present
/// with fat-envelope fallback: the fat-only test vetoed loop webs whose
/// dest reg also held a loop-spanning value (p20_sum_i64: v21's fat
/// (13,61) blocked both the v27 and v50 moves although no segment
/// overlaps, +8 movdqas).  Soundness: segments cover every live point
/// (the same basis the repair verifier reasons about), so a true overlap
/// is never missed — only false ones are removed.
fn fp_phi_move_conflicts(
    seg_cov: &FxHashMap<u32, Vec<(u32, u32)>>,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    assignments: &FxHashMap<u32, PhysReg>,
    phi_dest: u32,
    backedge_src: u32,
    d_reg: u8,
) -> bool {
    let src_cov: &[(u32, u32)] = match seg_cov.get(&backedge_src) {
        Some(pieces) => pieces,
        None => match iv_map.get(&backedge_src) {
            Some(fat) => std::slice::from_ref(fat),
            // No coverage at all: fail closed (the caller also gates on a
            // fat interval, so this is unreachable in practice).
            None => return true,
        },
    };
    if src_cov.is_empty() {
        return true;
    }
    assignments.iter().any(|(&vid, &home)| {
        if home.0 != d_reg || vid == backedge_src || vid == phi_dest {
            return false;
        }
        match seg_cov.get(&vid) {
            Some(pieces) => pieces
                .iter()
                .any(|&p| src_cov.iter().any(|&s| intervals_overlap(p, s))),
            None => iv_map
                .get(&vid)
                .is_some_and(|&f| src_cov.iter().any(|&s| intervals_overlap(f, s))),
        }
    })
}

/// True when none of `pieces` overlaps any already-held piece on a register,
/// under the allocator's half-open convention (`intervals_overlap`): a piece
/// ending at the exact point another begins shares cleanly (the in-place
/// boundary handoff).
fn sse_chain_pieces_fit(held: &[(u32, u32)], pieces: &[(u32, u32)]) -> bool {
    held.iter()
        .all(|&h| pieces.iter().all(|&p| !intervals_overlap(h, p)))
}

/// Destructive-form vector register allocation (see the call site for the
/// rationale).
///
/// Coverage comes from the real liveness result (`owned_live_segments`:
/// hole-aware segments, fat-envelope fallback) — never from first/last
/// mention points.  The mention model had two under-approximation holes,
/// both closed here:
///
/// * Copy/Phi/Call *definitions* were never recorded, so a value defined
///   by a phi-edge Copy and first used later had a span starting at its
///   first *use*; a short-lived chain value in between could claim the
///   same register and its definition clobbered the accumulator (P0).
/// * Terminator uses were never recorded, so a span could end early and
///   the register recycled before the terminator read it.
///
/// Sharing rule: two candidates share a register only when their segment
/// sets are disjoint under `intervals_overlap`.  The in-place *preference*
/// (a destination takes its dying first operand's register) additionally
/// requires a proven death: the operand's exact last read — operands of
/// every instruction and terminator, the complete read model for vector
/// values — is the defining instruction itself.  Vector values are never
/// GEP bases, never `for_each_value_use` extras, and never inline-asm
/// outputs; any candidate touching those paths (all fail-closed here) or
/// recorded in `folded_read_points` is vetoed out of handoffs.  When the
/// segments overhang past the proven death (block-granular
/// over-approximation), the preference simply fails the overlap test and
/// the destination takes another free register: sound in every build,
/// optimal whenever the segments are tight.  Boundary handoffs are
/// repair-clean — the post-RA verifier uses the same overlap test, so a
/// handoff pair never trips an eviction.
///
/// Soundness: `sse_load_arg`/`vec_operand_reg`/`vex128_source` only ever
/// read a home at a use point, and the phi Copy lowering (`emit_copy_value`)
/// re-establishes homes across block boundaries with `movdqa`, so a home is
/// valid wherever the value is live.  The pool holds caller-saved XMM only,
/// hence the per-segment call-spanning veto; the scratch registers
/// (xmm0/xmm1, xmm2 for pblendvb/VNNI) never enter this pool (call site
/// filters `>= 21`).
/// Entry vector-parameter loads (`x86_vector_param_loads`) whose incoming
/// ABI register is the vector RETURN register (%xmm0/%ymm0).  The
/// destructive FMA handoff prefers them when the result is returned, so the
/// copy peephole can collapse the whole function onto %ymm0.  Empty off
/// x86-64.
fn x86_return_register_param_loads(func: &IrFunction) -> FxHashSet<u32> {
    let mut out = FxHashSet::default();
    if crate::common::types::target_elf_machine() != 62 {
        return out;
    }
    let loads = crate::backend::stack_layout::x86_vector_param_loads(func);
    if !loads.iter().any(Option::is_some) {
        return out;
    }
    let classes = crate::backend::call_abi::classify_params(
        func,
        &crate::backend::call_abi::CallAbiConfig::sysv_x86_64(),
    );
    for (idx, load) in loads.iter().enumerate() {
        if let (
            Some((v, _)),
            Some(crate::backend::call_abi::ParamClass::F128SseReg { reg_idx: 0, .. }),
        ) = (load, classes.get(idx))
        {
            out.insert(v.0);
        }
    }
    out
}

/// Values returned directly by some `Return` terminator of `func`.
fn returned_vector_values(func: &IrFunction) -> FxHashSet<u32> {
    func.blocks
        .iter()
        .filter_map(|b| match &b.terminator {
            Terminator::Return(Some(Operand::Value(v))) => Some(v.0),
            _ => None,
        })
        .collect()
}

fn allocate_vector_registers_destructive(
    func: &IrFunction,
    candidates: &FxHashSet<u32>,
    pool: &[PhysReg],
    taken: &dyn Fn(u32) -> bool,
    liveness: &LivenessResult,
) -> Vec<(u32, PhysReg)> {
    if pool.is_empty() || candidates.is_empty() {
        return Vec::new();
    }
    if sse_chain_debug_enabled() {
        let pool_ids: Vec<u8> = pool.iter().map(|r| r.0).collect();
        let mut taken_ids: Vec<u32> = candidates.iter().copied().filter(|v| taken(*v)).collect();
        taken_ids.sort_unstable();
        eprintln!(
            "[SSE-CHAIN] func={} pool={pool_ids:?} taken={taken_ids:?}",
            func.name
        );
    }
    // Coverage: hole-aware segments where present, fat-envelope fallback.
    // Chain values never join a GPR coalesce web, so the owner map is empty
    // and every value owns exactly its own pieces.
    let no_webs: FxHashMap<u32, u32> = FxHashMap::default();
    let owned = owned_live_segments(liveness, &no_webs);

    // Pass 1: exact last-read points + hidden-use vetoes.  Same point
    // scheme as liveness (layout order, +1 per instruction, +1 per
    // terminator) so these coordinates compare directly against segment
    // endpoints.  Instruction + terminator operands are the complete read
    // model for vector values: anything else (`for_each_value_use` extras,
    // inline-asm outputs) vetoes the value out of handoffs, fail-closed.
    let mut last_read: FxHashMap<u32, u32> = FxHashMap::default();
    let mut hidden: FxHashSet<u32> = FxHashSet::default();
    let mut point: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if candidates.contains(&v.0) {
                        last_read.insert(v.0, point);
                    }
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                if candidates.contains(&v.0) {
                    hidden.insert(v.0);
                }
            });
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, v, _) in outputs {
                    if candidates.contains(&v.0) {
                        hidden.insert(v.0);
                    }
                }
            }
            point = point.saturating_add(1);
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if candidates.contains(&v.0) {
                    last_read.insert(v.0, point);
                }
            }
        });
        point = point.saturating_add(1);
    }

    // Homeable set: live coverage, not already homed, not held across a
    // call (the pool is caller-saved XMM).  Values with no coverage at all
    // are skipped, fail-closed.
    let mut coverage: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for &v in candidates {
        if taken(v) {
            continue;
        }
        let Some(pieces) = owned.get(&v) else {
            continue;
        };
        if pieces.is_empty() {
            continue;
        }
        let spans_call = pieces.iter().any(|&(start, end)| {
            spans_any_call(
                &LiveInterval {
                    value_id: v,
                    start,
                    end,
                },
                &liveness.call_points,
            )
        });
        if spans_call {
            continue;
        }
        // Proof witnesses, checked in test builds: a chain value is a
        // 128-bit vector, never a folded GEP base (pointers) and never the
        // target of a hidden folded re-read — the operand walk above is a
        // complete read model for handoff purposes.
        debug_assert!(
            !liveness.gep_base_values.contains(&v),
            "sse128 chain value {v} is a folded GEP base"
        );
        debug_assert!(
            !liveness.folded_read_points.contains_key(&v),
            "sse128 chain value {v} has hidden folded reads"
        );
        coverage.insert(v, pieces.clone());
    }
    if coverage.is_empty() {
        return Vec::new();
    }
    let debug = sse_chain_debug_enabled();
    if debug {
        let mut vs: Vec<u32> = coverage.keys().copied().collect();
        vs.sort_unstable();
        eprintln!("[SSE-CHAIN] func={} coverage:", func.name);
        for x in &vs {
            eprintln!(
                "[SSE-CHAIN]   v{x} pieces={:?} last_read={:?} hidden={}",
                coverage[x],
                last_read.get(x),
                hidden.contains(x)
            );
        }
    }

    let ret_reg_param_loads = x86_return_register_param_loads(func);
    let returned_values = returned_vector_values(func);

    // Pass 2: destructive handoff edges.  At the chain instruction defining
    // `d` from first operand `a0`, the edge `d <- a0` is proven when `a0`
    // is never read again afterwards (its exact last read is this very
    // instruction) and no other operand aliases it.  The emitter's in-place
    // form (`op %src, %dst` with dst == a0's home) then overwrites only a
    // dead value.
    let mut death_from: FxHashMap<u32, u32> = FxHashMap::default();
    let mut point: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                dest: Some(d),
                args,
                op,
                ..
            } = inst
            {
                if is_sse128_chain_op(op) && coverage.contains_key(&d.0) {
                    // FMA3 is destructive in ANY operand position — the
                    // emitter re-encodes the 132/231 family for whichever
                    // operand the destination home holds, and every form
                    // reads all three sources before the write, so an
                    // aliased operand (`a*a + c`, `a*b + a`) is exact too.
                    // Two-operand SSE forms overwrite only the first operand
                    // and need it unaliased.
                    // Among several dying FMA operands: when the result is
                    // RETURNED, the parameter that arrived in the return
                    // register wins (dest ← its home; the copy peephole then
                    // collapses prestore + result + return move onto %ymm0 —
                    // `a*b + c` becomes the single `vfmadd132ps`).  Otherwise
                    // the operand defined LAST wins (the chain temporary, a
                    // broadcast, the previous FMA): an entry parameter dies
                    // here just as well, but its incoming ABI register is
                    // better left alone (`mix_sab`: dest ← broadcast home,
                    // retargeted to %ymm0 by the peephole; dest ← param home
                    // would cost the prestore copy plus the return move).
                    let fma3 = is_fma3_op(op);
                    let positions = if fma3 { args.len() } else { 1.min(args.len()) };
                    let returned = returned_values.contains(&d.0);
                    let mut best: Option<(u32, u32)> = None; // (rank, value)
                    for (pos, arg) in args.iter().take(positions).enumerate() {
                        let Operand::Value(a) = arg else { continue };
                        let reused = !fma3
                            && args[pos + 1..]
                                .iter()
                                .any(|o| matches!(o, Operand::Value(v) if v.0 == a.0));
                        if reused
                            || hidden.contains(&a.0)
                            || liveness.folded_read_points.contains_key(&a.0)
                            || last_read.get(&a.0) != Some(&point)
                        {
                            continue;
                        }
                        let Some(pieces) = coverage.get(&a.0) else {
                            continue;
                        };
                        let def = pieces.iter().map(|p| p.0).min().unwrap_or(0);
                        let rank = if returned && ret_reg_param_loads.contains(&a.0) {
                            u32::MAX
                        } else {
                            def
                        };
                        if best.is_none_or(|(br, _)| rank > br) {
                            best = Some((rank, a.0));
                        }
                    }
                    if let Some((_, a)) = best {
                        death_from.insert(d.0, a);
                    }
                }
            }
            point = point.saturating_add(1);
        }
        point = point.saturating_add(1);
    }

    // Linear scan in (start, end, id) order over hole-aware coverage.
    // `held` per register accumulates every holder's pieces, so values in
    // each other's holes share the register; `lru_end` prefers the most
    // recently freed register to preserve long-free ones for long spans.
    // Deterministic: sorted order, pool-order iteration, and `max_by_key`
    // (last-maximum) ties.
    let mut order: Vec<u32> = coverage.keys().copied().collect();
    order.sort_by_key(|v| {
        let pieces = &coverage[v];
        (
            pieces.first().map(|&(s, _)| s).unwrap_or(u32::MAX),
            pieces.last().map(|&(_, e)| e).unwrap_or(u32::MAX),
            *v,
        )
    });

    if debug {
        let mut es: Vec<(u32, u32)> = death_from.iter().map(|(&d, &a)| (d, a)).collect();
        es.sort_unstable();
        eprintln!(
            "[SSE-CHAIN] func={} handoffs={es:?} order={order:?}",
            func.name
        );
    }

    let mut held: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
    let mut lru_end: FxHashMap<u8, u32> = FxHashMap::default();
    let mut assigned: Vec<(u32, PhysReg)> = Vec::new();
    let mut assigned_map: FxHashMap<u32, PhysReg> = FxHashMap::default();
    for v in order {
        let pieces = &coverage[&v];
        let fits = |rn: u8| {
            held.get(&rn)
                .is_none_or(|h| sse_chain_pieces_fit(h, pieces))
        };
        // Prefer the proven-dead first operand's register when it fits:
        // the preference failing the overlap test (segment overhang past
        // the proven death) is the automatic veto — the destination then
        // takes another free register instead of tripping a repair
        // eviction later.
        let preference = death_from
            .get(&v)
            .and_then(|a0| assigned_map.get(a0))
            .map(|r| r.0)
            .filter(|&rn| fits(rn));
        // Pending-handoff reservation: a handoff source whose target is not
        // yet assigned reserves its register.  A value without a usable
        // preference takes another free register when one fits, so an
        // unrelated value never steals a proven in-place home
        // (p20_sum_i64: v21 stole v47's xmm15 on an LRU tie and blocked the
        // v56 handoff, +11 movdqas).  Pure preference — soundness still
        // rests on the overlap test alone.
        let mut reserved: FxHashSet<u8> = FxHashSet::default();
        if preference.is_none() {
            for (&d, &a0) in death_from.iter() {
                if !assigned_map.contains_key(&d)
                    && let Some(&reg) = assigned_map.get(&a0)
                {
                    reserved.insert(reg.0);
                }
            }
        }
        let chosen = if let Some(rn) = preference {
            pool.iter().find(|r| r.0 == rn).copied()
        } else {
            let free: Vec<PhysReg> = pool.iter().filter(|r| fits(r.0)).copied().collect();
            free.iter()
                .filter(|r| !reserved.contains(&r.0))
                .max_by_key(|r| lru_end.get(&r.0).copied().unwrap_or(0))
                .or_else(|| {
                    free.iter()
                        .max_by_key(|r| lru_end.get(&r.0).copied().unwrap_or(0))
                })
                .copied()
        };
        if debug {
            eprintln!("[SSE-CHAIN]   v{v} pref={preference:?} chosen={chosen:?} pieces={pieces:?}");
        }
        if let Some(reg) = chosen {
            held.entry(reg.0).or_default().extend_from_slice(pieces);
            let end = pieces.last().map(|&(_, e)| e).unwrap_or(0);
            lru_end
                .entry(reg.0)
                .and_modify(|e| *e = (*e).max(end))
                .or_insert(end);
            assigned_map.insert(v, reg);
            assigned.push((v, reg));
        }
        // No free register: the value keeps its fallback home (stack slot,
        // staged through xmm0/xmm1 by the generic emitters).
    }
    assigned
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
                // Inline-expanded memcpy/memset (x86-64) stage their operands
                // hazard-free for any home: memset reads the fill value into
                // %rax (never a home) before the destination; memcpy orders
                // the two pointer loads by their homes (`stage_copy_operands`).
                // Excluding their operands from the argument registers only
                // spilled parameters and bought a frame.
                Instruction::Call { func: name, info }
                    if crate::common::types::target_elf_machine()
                        == crate::backend::elf::EM_X86_64
                        && (crate::backend::generation::inline_memcpy_len(name, info)
                            .is_some()
                            || crate::backend::generation::x86_inline_memset_len(name, info)
                                .is_some()) =>
                {
                    continue;
                }
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

    // A Copy dest fed by `Const(I64)` on a 32-bit target is ambiguous:
    // `IrConst::from_i64` stores U32/U16/U8 constants zero-extended in an
    // `I64` payload, so the spelling cannot say whether the value is a real
    // 64-bit local (two-register / 64-bit slot territory) or a 32-bit value
    // in unsigned-safe storage. The value's own width evidence decides
    // (`compute_value_type_map` is the single source of truth); with no
    // evidence, keep the historical conservative non-GPR classification.
    // Without this, `u32 err = 0;` in a loop — lowered to
    // `Copy v <- Const(I64(0))` by phi elimination + const folding — was
    // excluded from GPR allocation for the whole function (and its copy web
    // poisoned via `propagate_copy_web`), the root cause of the boot
    // corpus's store-to-frame-slot traffic.
    let value_type_map = crate::backend::common::compute_value_type_map(func);

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
                    if !cast_result_is_gpr(*from_ty, *to_ty, is_32bit) {
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
                Instruction::Phi { dest, ty, .. } => {
                    if is_non_gpr_type(ty, is_32bit) {
                        non_gpr_values.insert(dest.0);
                    }
                }
                Instruction::Copy { dest, src } => {
                    let src_is_non_gpr = match src {
                        Operand::Const(IrConst::F32(_))
                        | Operand::Const(IrConst::F64(_))
                        | Operand::Const(IrConst::LongDouble(..))
                        | Operand::Const(IrConst::I128(_))
                        // Decimal BID carriers are FP-class (`IrType::is_float`):
                        // a D64 copy in a GPR loses its high word on i686.
                        | Operand::Const(IrConst::D32(_))
                        | Operand::Const(IrConst::D64(_)) => true,
                        // Ambiguous storage form: trust the value's own
                        // width evidence over the constant's spelling
                        // (see the value_type_map comment above).
                        Operand::Const(IrConst::I64(_)) if is_32bit => {
                            !value_type_map.get(&dest.0).is_some_and(|t| t.size() <= 4)
                        }
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
                            | IntrinsicOp::FmaScalarF64Signed(..)
                            | IntrinsicOp::FmaScalarF32Signed(..)
                            | IntrinsicOp::RoundScalarF64(_)
                            | IntrinsicOp::RoundScalarF32(_)
                            | IntrinsicOp::CopysignF64
                            | IntrinsicOp::CopysignF32
                            | IntrinsicOp::FixedDistanceF32x8
                            | IntrinsicOp::FixedDistanceF64x4
                            // FP horizontal reductions produce scalar
                            // F32/F64 SSA values in the SSE domain
                            // (store_xmm_to).  They must reach the XMM scan
                            // (f64_intervals filters on this set BEFORE
                            // f64_value_set): without a non-GPR admission
                            // the combine result falls back to a slot home
                            // and the reduction tail pays a store+reload
                            // pair (p16/p23 slot bounce — follow-up #3).
                            | IntrinsicOp::HorizontalAddF64x4
                            | IntrinsicOp::HorizontalAddF64x2
                            | IntrinsicOp::VecHorizontalAddF64x4
                            | IntrinsicOp::VecHorizontalAddF64x2
                            // StrictRecipMulAdd is scalar F64 despite doing
                            // AVX2 work internally; its ordered accumulator
                            // must use the normal XMM scalar allocation path.
                            | IntrinsicOp::StrictRecipMulAddF64x4
                            | IntrinsicOp::StrictRecipMulAddPackedF64x4
                            | IntrinsicOp::VecHorizontalAddF32x8
                            | IntrinsicOp::VecHorizontalAddF32x4
                    ) || op.produces_vector_value()
                        // Scalar intrinsic result typing is shared with the
                        // FP scan. A lane extract must pass BOTH filters;
                        // admitting it only in collect_f64_values is inert.
                        || inst.result_type().as_ref().is_some_and(is_scalar_fp)
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
            O::VecZeroF32x4
            | O::VecLoadF32x4
            | O::VecAddF32x4
            | O::VecMulF32x4
            | O::VecFmaF32x4
            | O::VecFnmaF32x4
            | O::VecFmsF32x4
            | O::VecFnmsF32x4 => Some(3),
            O::VecZeroF64x2
            | O::VecLoadF64x2
            | O::VecAddF64x2
            | O::VecMulF64x2
            | O::VecFmaF64x2
            | O::VecFnmaF64x2
            | O::VecFmsF64x2
            | O::VecFnmsF64x2 => Some(4),
            O::VecZeroI32x8 | O::VecLoadI32x8 | O::VecAddI32x8 | O::VecMulI32x8 => Some(5),
            // BB-SLP 256-bit dword shifts: class 5 (I32x8 family; the
            // home-aware immediate-shift emitter reads/writes the YMM
            // home directly).
            O::VecShlI32x8 | O::VecLShrI32x8 | O::VecAShrI32x8 => Some(5),
            // Per-lane variable dword shifts: two I32x8 operands, I32x8
            // result (`emit_avx_binary_256`, like the sub/bitwise ops).
            O::VecShlvI32x8 | O::VecLShrvI32x8 | O::VecAShrvI32x8 => Some(5),
            // v12 Fix F: Max reduction producers. VecBroadcastI32x8 seeds the
            // max accumulator (init = arr[0] broadcast), VecMaxI32x8 produces
            // the new accumulator each iteration. Classifying them as class 5
            // (I32x8) lets the Copy-web connect the backedge so the accumulator
            // stays register-homed (no per-iter stack round-trip, which made
            // the vectorized find_max SLOWER than scalar).
            O::VecBroadcastI32x8 | O::VecMaxI32x8 | O::VecMinI32x8 => Some(5),
            O::VecZeroI32x4 | O::VecLoadI32x4 | O::VecAddI32x4 | O::VecMulI32x4 => Some(6),
            // HALF-WIDE dword-pair endpoints: the value web is the
            // I32x4 family (the intermediate ops ARE the I32x4
            // intrinsics), so the load/store/gather endpoints join
            // class 6 to keep the web connected.
            O::VecLoadI32x4Pair | O::VecStoreI32x4Pair | O::VecPackI32x4Pair => Some(6),
            // ARX lane ops (rotate/shuffle): class 6 (I32x4 family).
            O::VecRotlI32x4 | O::VecShufdI32x4 | O::VecXorI32x4 | O::VecOrI32x4 => Some(6),
            // BB-SLP 128-bit dword shifts: class 6 (I32x4 family).
            O::VecShlI32x4 | O::VecLShrI32x4 | O::VecAShrI32x4 => Some(6),
            // Per-lane variable dword shifts (VEX.128 three-operand form,
            // `emit_vex_binary_128`): class 6 (I32x4 family).
            O::VecShlvI32x4 | O::VecLShrvI32x4 | O::VecAShrvI32x4 => Some(6),
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
            // The byte-predicate counting reduction family (I64x4):
            // `vpsadbw` produces the four group counts, `vpaddq` updates
            // the u64x4 accumulator, and the zero seeds it.  Class 8 so
            // the Copy-web can home the loop-carried accumulator.
            O::VecSadbwU8x32 | O::VecAddI64x4 | O::VecZeroI64x4 => Some(8),
            // Adler-32 epic: the maddubs result and the .rodata constant
            // tables (weights, ones, seed mask) are 256-bit YMM values kept
            // register-resident across the vector loop — class 8 (the
            // I64x4 physical family) so the Copy-web can home them.  The
            // maddwd result feeds the class-5 dword accumulator add.
            O::VecMaddubsU8x32 | O::VecConstI8x32 => Some(8),
            O::VecMaddwdI16x16 => Some(5),
            // BB-SLP I64x4 copy/ALU chains: loads, sub/bitwise, splat.
            O::VecLoadI64x4
            | O::VecSubI64x4
            | O::VecAndI64x4
            | O::VecOrI64x4
            | O::VecXorI64x4
            | O::VecBroadcastI64x4 => Some(8),
            // BB-SLP 256-bit qword shifts: class 8 (I64x4 family).
            O::VecShlI64x4 | O::VecLShrI64x4 => Some(8),
            // Per-lane variable qword shifts: class 8 (I64x4 family).
            O::VecShlvI64x4 | O::VecLShrvI64x4 => Some(8),
            // BB-SLP I64x2 bitwise + gather.
            O::VecAndI64x2 | O::VecOrI64x2 | O::VecXorI64x2 | O::VecPackI64x2 => Some(7),
            // BB-SLP 128-bit qword shifts: class 7 (I64x2 family).
            O::VecShlI64x2 | O::VecLShrI64x2 => Some(7),
            // Per-lane variable qword shifts: class 7 (I64x2 family).
            O::VecShlvI64x2 | O::VecLShrvI64x2 => Some(7),
            // BB-SLP F64x2 gather.
            O::VecPackF64x2 => Some(4),
            // BB-SLP I32x4 gather.
            O::VecPackI32x4 => Some(6),
            // BB-SLP byte/halfword copy + ALU chains (128-bit).
            O::VecLoadI8x16
            | O::VecAddI8x16
            | O::VecSubI8x16
            | O::VecAndI8x16
            | O::VecOrI8x16
            | O::VecXorI8x16 => Some(9),
            O::VecLoadI16x8
            | O::VecAddI16x8
            | O::VecSubI16x8
            | O::VecMulI16x8
            | O::VecAndI16x8
            | O::VecOrI16x8
            | O::VecXorI16x8 => Some(10),
            // BB-SLP 128-bit word shifts: class 10 (I16x8 family).
            O::VecShlI16x8 | O::VecLShrI16x8 | O::VecAShrI16x8 => Some(10),
            _ => None,
        }
    };
    let legal_consumer = |op: &O, class: u8| -> bool {
        match class {
            1 => matches!(
                op,
                O::VecAddF32x8 | O::VecMulF32x8 | O::VecFmaF32x8 | O::VecHorizontalAddF32x8
                    // BB-SLP 256-bit lane extract: reads the YMM home
                    // through its XMM alias / vextractf128; scratch is
                    // xmm1 only.
                    | O::VecExtractLaneF32x8
            ),
            2 => matches!(
                op,
                O::VecAddF64x4 | O::VecMulF64x4 | O::VecFmaF64x4 | O::VecHorizontalAddF64x4
                    // BB-SLP 256-bit lane extract: reads the YMM home
                    // through its XMM alias / vextracti128; scratch is
                    // xmm1 only.
                    | O::VecExtractLaneF64x4
            ),
            3 => matches!(
                op,
                O::VecAddF32x4 | O::VecMulF32x4 | O::VecHorizontalAddF32x4
                    | O::VecFmaF32x4
                    | O::VecFnmaF32x4
                    | O::VecFmsF32x4
                    | O::VecFnmsF32x4
                    // BB-SLP lane extract (128-bit F32): reads the XMM
                    // home; scratch confined to xmm1.
                    | O::VecExtractLaneF32x4
            ),
            4 => matches!(
                op,
                O::VecAddF64x2 | O::VecMulF64x2 | O::VecHorizontalAddF64x2
                    | O::VecFmaF64x2
                    | O::VecFnmaF64x2
                    | O::VecFmsF64x2
                    | O::VecFnmsF64x2
                    // BB-SLP sinks: the 128-bit store reads a homed source
                    // directly (vec_store_source_128); the lane extract
                    // stages through xmm1 only (sse_load_arg discipline).
                    | O::VecStoreF64x2
                    | O::VecExtractLaneF64x2
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
                    | O::VecMinI32x8
                    | O::VecHorizontalMaxI32x8
                    | O::VecHorizontalMinI32x8
                    | O::VecMaskedAddI32x8
                    // BB-SLP 256-bit dword shifts consume the I32x8 home
                    // directly (three-operand VEX immediate form).
                    | O::VecShlI32x8
                    | O::VecLShrI32x8
                    | O::VecAShrI32x8
                    // Variable dword shifts: `emit_avx_binary_256` reads
                    // both homed sources in place.
                    | O::VecShlvI32x8
                    | O::VecLShrvI32x8
                    | O::VecAShrvI32x8
                    // BB-SLP 256-bit lane extract: reads the YMM home
                    // (half staging), scratch confined to xmm1.
                    | O::VecExtractLaneI32x8
                    // Integer map lane bitwise (And/Or/Xor): the counting
                    // reduction's `& 1` mask step goes through
                    // `emit_avx_binary_256` exactly like the add/mul arms
                    // above, so a register-homed I32x8 value (the 0x01
                    // byte splat, a compare mask) is safe to keep live
                    // across it.  Without this entry the splat's home was
                    // illegal, forcing the `movdqa %ymm5, %ymm0` +
                    // `movdqa %ymm3, %ymm1` staging pair before every
                    // `vpand` — two dead copies per counting iteration.
                    | O::VecAndI32x8
                    | O::VecOrI32x8
                    | O::VecXorI32x8
            ),
            6 => matches!(
                op,
                O::StrictRecipMulAddPackedF64x4
                    | O::VecAddI32x4 | O::VecMulI32x4 | O::VecHorizontalAddI32x4
                    // ARX lane ops: their emitters resolve a homed source
                    // through %xmm0/%xmm1 scratch without writing the home
                    // (movdqa in, shifts/por on scratch), so a register-homed
                    // I32x4 value stays live across them.
                    | O::VecRotlI32x4
                    | O::VecShufdI32x4
                    | O::VecXorI32x4
                    | O::VecOrI32x4
                    // BB-SLP 128-bit dword shifts: VEX.128 immediate
                    // form reads the homed source in place.
                    | O::VecShlI32x4
                    | O::VecLShrI32x4
                    | O::VecAShrI32x4
                    // Variable dword shifts: `emit_vex_binary_128` reads
                    // both homed sources in place (`vex128_source`).
                    | O::VecShlvI32x4
                    | O::VecLShrvI32x4
                    | O::VecAShrvI32x4
                    // The ARX pass's exit materialization consumes the
                    // loop-carried state through a 128-bit store; the
                    // register-home store path (vec_store_source_128)
                    // reads the homed register directly.
                    | O::VecStoreI32x4
                    // HALF-WIDE dword-pair store: same register-home
                    // source path (vec_store_source_128).
                    | O::VecStoreI32x4Pair
                    // BB-SLP lane extract: xmm1-only staging.
                    | O::VecExtractLaneI32x4
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
                    // BB-SLP sinks: 128-bit store (register-home source)
                    // and lane extract (xmm1-only staging).
                    | O::VecStoreI64x2
                    | O::VecExtractLaneI64x2
                    // BB-SLP bitwise consumes a homed I64x2 through the
                    // emit_sse_binary_128 home/memfold paths.
                    | O::VecAndI64x2
                    | O::VecOrI64x2
                    | O::VecXorI64x2
                    // BB-SLP 128-bit qword shifts: same home-aware
                    // immediate-shift contract as the bitwise consumes.
                    | O::VecShlI64x2
                    | O::VecLShrI64x2
                    // Variable qword shifts: same `emit_vex_binary_128`
                    // home-reading contract.
                    | O::VecShlvI64x2
                    | O::VecLShrvI64x2
            ),
            // I64x4 counting family: the sad partials are consumed by the
            // accumulator add, and the accumulator itself by the horizontal
            // exit — both read their sources through the home-aware
            // emitters (`emit_avx_binary_256` / the horizontal chain).
            8 => matches!(
                op,
                O::VecAddI64x4 | O::VecHorizontalAddI64x4 | O::VecSadbwU8x32
                    // Adler-32 epic multiply-adds: both consume their
                    // 256-bit sources through `emit_avx_binary_256`'s
                    // home-aware paths (all-homed register form, slot r/m
                    // source, or staged scratch — never a home write), so a
                    // register-homed weights/ones/maddubs value stays live
                    // across them exactly like the counting binaries.
                    | O::VecMaddubsU8x32
                    | O::VecMaddwdI16x16
                    // BB-SLP 4×u64 copy chain: store reads the homed YMM
                    // directly (vec_store_source_256); sub/bitwise go
                    // through emit_avx_binary_256's home-aware paths.
                    | O::VecStoreI64x4
                    | O::VecSubI64x4
                    | O::VecAndI64x4
                    | O::VecOrI64x4
                    | O::VecXorI64x4
                    // 256-bit lane extract: same home-reading contract.
                    | O::VecExtractLaneI64x4
                    // BB-SLP 256-bit qword shifts: same home-aware
                    // immediate-shift contract as the sub/bitwise ops.
                    | O::VecShlI64x4
                    | O::VecLShrI64x4
                    // Variable qword shifts: same `emit_avx_binary_256`
                    // home-reading contract as the sub/bitwise ops.
                    | O::VecShlvI64x4
                    | O::VecLShrvI64x4
            ),
            // BB-SLP byte/halfword chains: store (vec_store_source_128
            // register-home path), add/sub/bitwise (emit_sse_binary_128
            // home-aware paths).
            9 => matches!(
                op,
                O::VecStoreI8x16
                    | O::VecAddI8x16
                    | O::VecSubI8x16
                    | O::VecAndI8x16
                    | O::VecOrI8x16
                    | O::VecXorI8x16
            ),
            10 => matches!(
                op,
                O::VecStoreI16x8
                    | O::VecAddI16x8
                    | O::VecSubI16x8
                    | O::VecMulI16x8
                    | O::VecAndI16x8
                    | O::VecOrI16x8
                    | O::VecXorI16x8
                    // BB-SLP halfword lane extract: `pextrw` from the
                    // staged XMM source — reads the home, no write.
                    | O::VecExtractLaneI16x8
                    // BB-SLP 128-bit word shifts: VEX.128 immediate
                    // form reads the homed source in place.
                    | O::VecShlI16x8
                    | O::VecLShrI16x8
                    | O::VecAShrI16x8
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
                                // Byte-count/Adler-32 epic exits: the
                                // 4×I64 horizontal reduce (vextracti128 +
                                // vpaddq + vpshufd chain) confines scratch
                                // to the reserved ymm0/ymm1 pair and only
                                // READS the accumulator's home — the same
                                // discipline as the I32x8 twin above.
                                // Without this entry the epic's exit
                                // poisoned EVERY vector web in the
                                // function (the counting accumulator
                                // round-tripped 32 bytes through the
                                // stack every iteration).
                                | O::VecHorizontalAddI64x4
                                // v12 Fix C: widening reductions consume an
                                // I64x2 accumulator (whitelisted above). Their
                                // lowerings confine scratch to xmm0/xmm1 so an
                                // XMM-homed accumulator is safe — do NOT let
                                // them poison the whole function.
                                | O::VecWidenAddI32x4ToI64x2
                                | O::VecWidenMaskedAddI32x4ToI64x2
                                // StrictRecipMulAddF64x4 owns only the
                                // reserved xmm0/xmm1 scratch pair and has no
                                // vector SSA result. It therefore cannot
                                // poison an unrelated mature vector
                                // accumulator web in the same function.
                                | O::StrictRecipMulAddF64x4
                                | O::StrictRecipMulAddPackedF64x4
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
                                | O::VecBroadcastI8x32
                                | O::VecBroadcastI16x16
                                | O::VecStoreI32x8
                                | O::VecStoreI32x4
                                | O::VecStoreF32x8
                                | O::VecStoreF32x4
                                | O::VecStoreF64x4
                                | O::VecStoreF64x2
                                | O::VecStoreI64x2
                                | O::VecStoreI8x32
                                | O::VecStoreI16x16
                                | O::VecLoadI8x32
                                | O::VecLoadI16x16
                                // v12 Fix C: max reductions (find_max) and
                                // their horizontal reduce — lowerings
                                // confine scratch to xmm0/xmm1, exempt them.
                                | O::VecMaxI32x8
                                | O::VecMinI32x8
                                | O::VecHorizontalMaxI32x8
                                | O::VecHorizontalMinI32x8
                                | O::VecHorizontalMaxI32x4
                                | O::VecSmaxI32x4
                                | O::VecSminI32x4
                                | O::VecLoadWidenI32ToI64x2
                                // Byte-predicate counting loops carry the
                                // map-family byte ops (compare masks, the
                                // `& 1` and, blends) AROUND the I64x4
                                // accumulator. They are not accumulator
                                // producers, exactly like the broadcast/
                                // store twins above; without this
                                // exemption the poison check evicts the
                                // counting accumulator to a stack slot
                                // (one 32-byte round-trip per iteration).
                                | O::VecCmpI8x32
                                | O::VecAndI32x8
                                | O::VecOrI32x8
                                | O::VecXorI32x8
                                | O::VecAddI8x32
                                | O::VecSubI8x32
                                | O::VecBlendvI8x32
                                | O::VecMinU8x32
                                | O::VecMaxU8x32
                                | O::VecMinI8x32
                                | O::VecMaxI8x32
                                // 256-bit halfword map ops (OP-05g word
                                // lanes): same scratch-pair discipline as
                                // their byte twins — without the exemption
                                // they poison every vector web in the
                                // function back to protected slots.
                                | O::VecAddI16x16
                                | O::VecSubI16x16
                                | O::VecMulI16x16
                                | O::VecCmpI16x16
                                | O::VecMinI16x16
                                | O::VecMaxI16x16
                                | O::VecMinU16x16
                                | O::VecMaxU16x16
                                | O::VecBlendvI16x16
                                | O::VecAndI16x16
                                | O::VecOrI16x16
                                | O::VecXorI16x16
                                // BB-SLP sinks and lane extracts: no
                                // accumulator of their own; scratch confined
                                // to xmm0/xmm1 (or reading the home
                                // directly). Without the exemption they
                                // would poison every vector web in the
                                // function back to protected slots.
                                | O::VecStoreI64x4
                                | O::VecStoreI16x8
                                | O::VecStoreI8x16
                                | O::VecExtractLaneI32x4
                                | O::VecExtractLaneI64x2
                                | O::VecExtractLaneF64x2
                                | O::VecExtractLaneI64x4
                                | O::VecExtractLaneF64x4
                                | O::VecExtractLaneF32x4
                                | O::VecExtractLaneF32x8
                                | O::VecExtractLaneI32x8
                                | O::VecExtractLaneI16x8
                                | O::VecExtractLaneI16x16
                        ) =>
                {
                    if std::env::var("CCC_DEBUG_VECWEB").is_ok() {
                        eprintln!("[VECWEB-POISON] fn={} op={:?}", func.name, op);
                    }
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
    if std::env::var("CCC_DEBUG_VECWEB").is_ok() {
        let mut v: Vec<u32> = classes.keys().copied().collect();
        v.sort_unstable();
        eprintln!("[VECWEB-1] fn={} classes_after_prop={:?}", func.name, v);
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
        if std::env::var("CCC_DEBUG_VECWEB").is_ok() {
            let mut v: Vec<u32> = classes.keys().copied().collect();
            v.sort_unstable();
            eprintln!(
                "[VECWEB-2] fn={} evict={:?} remaining={:?}",
                func.name, bad, v
            );
        }
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
    if std::env::var("CCC_DEBUG_VECWEB").is_ok() {
        let mut v: Vec<u32> = classes.keys().copied().collect();
        v.sort_unstable();
        eprintln!("[VECWEB] fn={} web={:?}", func.name, v);
    }
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
            O::VecBroadcastI32x8 | O::VecBroadcastI8x32 | O::VecBroadcastI16x16 => Some(3),
            O::VecBroadcastF32x4 => Some(4),
            O::VecBroadcastF64x2 => Some(5),
            O::VecBroadcastI32x4 | O::VecBroadcastI8x16 | O::VecBroadcastI16x8 => Some(6),
            // Adler-32 epic invariants (see the consumer class 8 below):
            // the SAD zero and the .rodata weight/ones tables.
            O::VecZeroI64x4 | O::VecConstI8x32 => Some(8),
            _ => None,
        }
    };
    // Every consumer listed here must read a homed operand through the
    // register (avx_load_arg_to / sse_load_arg consult reg_assignments and
    // vec_live_regs first) or through vec_home_256/vec_home_128 — never raw
    // from the home slot.  A non-listed consumer strands the broadcast on
    // the stack (correct, just slower).
    let legal_consumer = |op: &O, class: u8| -> bool {
        match class {
            1 => matches!(
                op,
                O::VecMulF32x8
                    | O::VecAddF32x8
                    | O::VecSubF32x8
                    | O::VecDivF32x8
                    | O::VecXorF32x8
                    | O::VecSqrtF32x8
                    | O::VecRoundF32x8
                    | O::VecMaddF32x8
                    | O::VecMaddF32x8Signed(..)
                    | O::VecCmpF32x8
                    | O::VecBlendvF32x8
                    | O::VecMinF32x8
                    | O::VecMaxF32x8
            ),
            2 => matches!(
                op,
                O::VecMulF64x4
                    | O::VecAddF64x4
                    | O::VecSubF64x4
                    | O::VecDivF64x4
                    | O::VecXorF64x4
                    | O::VecSqrtF64x4
                    | O::VecRoundF64x4
                    | O::VecMaddF64x4
                    | O::VecMaddF64x4Signed(..)
                    | O::VecCmpF64x4
                    | O::VecBlendvF64x4
                    | O::VecMinF64x4
                    | O::VecMaxF64x4
            ),
            3 => matches!(
                op,
                O::VecMulI32x8
                    | O::VecAddI32x8
                    // Conditional-map consumers: a broadcast invariant is
                    // the compare rhs / blend true-arm (clamp shapes).
                    | O::VecCmpI32x8
                    | O::VecCmpI64x4
                    | O::VecBlendvI32x8
                    // Integer min/max maps: the broadcast is a clamp bound.
                    | O::VecMinI32x8
                    | O::VecMaxI32x8
                    // Bitwise lane maps: the broadcast is the operand of
                    // `x & K` / `x | K` / `x ^ K`, including the `mask & K`
                    // the select strength reduction produces.  These were
                    // missing, so a broadcast feeding a `vpand` was not
                    // recognised as a map broadcast, got no register home,
                    // and was re-read from the STACK every iteration.
                    | O::VecAndI32x8
                    | O::VecOrI32x8
                    | O::VecXorI32x8
                    | O::VecSubI32x8
                    | O::VecShlvI32x8
                    | O::VecLShrvI32x8
                    | O::VecAShrvI32x8
                    // Byte-lane maps share the 256-bit integer class:
                    // same YMM register file, same VecLoad/StoreI32x8
                    // endpoints, only the lane width of the ALU differs.
                    | O::VecAddI8x32
                    | O::VecSubI8x32
                 | O::VecAndI8x32
                    | O::VecOrI8x32
                    | O::VecXorI8x32
                    | O::VecCmpI8x32
                    | O::VecBlendvI8x32
                    | O::VecMinU8x32
                    | O::VecMaxU8x32
                    | O::VecMinI8x32
                    | O::VecMaxI8x32
                    // Word lanes (OP-05g) share the 256-bit integer class:
                    // same YMM file, same VecLoad/StoreI32x8 endpoints.
                    | O::VecAddI16x16
                    | O::VecSubI16x16
                    | O::VecMulI16x16
                    | O::VecCmpI16x16
                    | O::VecMinI16x16
                    | O::VecMaxI16x16
                    | O::VecMinU16x16
                    | O::VecMaxU16x16
                    | O::VecBlendvI16x16
                    // Halfword bitwise maps — the word twins of the byte
                    // entries above. Missing here, a `x & K` halfword
                    // broadcast got no register home and re-read from the
                    // stack every iteration (same defect class as the
                    // dword fix above; Review F2).
                    | O::VecAndI16x16
                    | O::VecOrI16x16
                    | O::VecXorI16x16
            ),
            4 => matches!(
                op,
                O::VecMulF32x4
                    | O::VecAddF32x4
                    | O::VecSubF32x4
                    | O::VecDivF32x4
                    | O::VecSqrtF32x4
                    | O::VecRoundF32x4
                    | O::VecCmpF32x4
                    | O::VecBlendvF32x4
                    | O::VecMinF32x4
                    | O::VecMaxF32x4
            ),
            5 => matches!(
                op,
                O::VecMulF64x2
                    | O::VecAddF64x2
                    | O::VecSubF64x2
                    | O::VecDivF64x2
                    | O::VecSqrtF64x2
                    | O::VecRoundF64x2
                    | O::VecCmpF64x2
                    | O::VecBlendvF64x2
                    | O::VecMinF64x2
                    | O::VecMaxF64x2
                    | O::VecFmaF64x2
                    | O::VecFnmaF64x2
                    | O::VecFmsF64x2
                    | O::VecFnmsF64x2
            ),
            6 => matches!(
                op,
                O::StrictRecipMulAddPackedF64x4
                    | O::VecMulI32x4
                    | O::VecAddI32x4
                    | O::VecCmpI32x4
                    | O::VecCmpI64x2
                    | O::VecBlendvI32x4
                    // Bitwise lane maps: the broadcast is the operand of
                    // `x & K` / `x | K` / `x ^ K`, including the `mask & K`
                    // the select strength reduction produces.  These were
                    // missing, so a broadcast feeding a `vpand` was not
                    // recognised as a map broadcast, got no register home,
                    // and was re-read from the STACK every iteration.
                    | O::VecAndI32x4
                    | O::VecOrI32x4
                    | O::VecXorI32x4
                    | O::VecSubI32x4
                    | O::VecShlvI32x4
                    | O::VecLShrvI32x4
                    | O::VecAShrvI32x4
                    | O::VecShlvI64x2
                    | O::VecLShrvI64x2
                    | O::VecAddI8x16
                    | O::VecSubI8x16
                    | O::VecCmpI8x16
                    | O::VecMinU8x16
                    | O::VecMaxU8x16
                    | O::VecBlendvI8x16
                    // Word lanes (OP-05g) share the 128-bit integer class.
                    | O::VecAddI16x8
                    | O::VecSubI16x8
                    | O::VecMulI16x8
                    | O::VecCmpI16x8
                    | O::VecMinI16x8
                    | O::VecMaxI16x8
                    | O::VecBlendvI16x8
            ),
            // Adler-32 epic invariants: the SAD zero operand
            // (`VecZeroI64x4`) and the .rodata weight/ones tables
            // (`VecConstI8x32`) are loop-invariant 256-bit values whose
            // ONLY uses are the epic's packed consumers — the same
            // charter as the broadcasts above (live across every
            // iteration, never an accumulator).  Without this class they
            // took no register home and every maddubs/maddwd/sadbw
            // re-read them from the stack.
            8 => matches!(
                op,
                O::VecSadbwU8x32
                    | O::VecMaddubsU8x32
                    | O::VecMaddwdI16x16
                    | O::VecAddI64x4
                    | O::VecSubI64x4
                    | O::VecAndI64x4
                    | O::VecOrI64x4
                    | O::VecXorI64x4
                    | O::VecShlvI64x4
                    | O::VecLShrvI64x4
            ),
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
                    // Address-side / pointer-only position (intrinsic
                    // dest_ptr, Store/Load ptr, GEP base, ...): never a
                    // packed register consumer.  Unconditional, fail-closed.
                    bad.insert(v.0);
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

/// Copy webs (mem2reg/vec_ssa phi lowering: `%m = copy %arm` in each
/// predecessor, the join reads `%m`): a Copy whose source is a candidate
/// makes its destination a candidate of the same class, to a fixpoint (a
/// web may chain through several joins).  Returns the destinations reached
/// from two different classes -- contradictory, poisoned by the caller.
fn propagate_copy_webs(func: &IrFunction, candidates: &mut FxHashMap<u32, u8>) -> FxHashSet<u32> {
    let mut web_conflict: FxHashSet<u32> = FxHashSet::default();
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } = inst
                else {
                    continue;
                };
                let Some(&class) = candidates.get(&src.0) else {
                    continue;
                };
                match candidates.get(&dest.0) {
                    None => {
                        candidates.insert(dest.0, class);
                        changed = true;
                    }
                    Some(&existing) if existing != class => {
                        web_conflict.insert(dest.0);
                    }
                    Some(_) => {}
                }
            }
        }
        if !changed {
            break;
        }
    }
    web_conflict
}

/// Poison whole Copy webs: every member of a web either keeps its home or
/// none does, so no transport copy ever pairs a homed source with a
/// slot-homed destination (or vice versa).
fn poison_copy_webs(func: &IrFunction, candidates: &FxHashMap<u32, u8>, bad: &mut FxHashSet<u32>) {
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } = inst
                else {
                    continue;
                };
                if !candidates.contains_key(&dest.0) || !candidates.contains_key(&src.0) {
                    continue;
                }
                if bad.contains(&dest.0) != bad.contains(&src.0) {
                    bad.insert(dest.0);
                    bad.insert(src.0);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Element-family class of a map-intermediate producer
/// (`collect_x86_map_intermediate_values`): `None` for every op that is not
/// a packed map producer.  Mirrors the broadcast/reduction classing so the
/// Copy-web and phi-merge machinery treats the classes uniformly.
fn x86_map_intrinsic_class(op: &crate::ir::intrinsics::IntrinsicOp) -> Option<u8> {
    use crate::ir::intrinsics::IntrinsicOp as O;
    match op {
        // The family zero seeds (`vec_ssa` reads of an indeterminate or
        // `{}`-initialised object): a `vxorps %ymmD,%ymmD,%ymmD` into
        // the home. Unclassified, the seed of `v32f s = {}; for (...)
        // s += p[i]` was stored to a 32-byte slot and reloaded into the
        // homed accumulator web — a dead frame plus two memory
        // operations per function for nothing.
        O::VecZeroF32x8
        | O::VecLoadF32x8
        | O::VecSubF32x8
        | O::VecDivF32x8
        | O::VecXorF32x8
        | O::VecSqrtF32x8
        | O::VecRoundF32x8
        | O::VecMaddF32x8
                | O::VecMaddF32x8Signed(..)
        | O::VecCmpF32x8
        | O::VecBlendvF32x8
        | O::VecMinF32x8
        | O::VecMaxF32x8
        // Add/Mul ride `emit_avx_binary_256`, the same audited emitter
        // as Sub/Div (W5: a streamed load feeding an add/mul chain was
        // rejected here and paid a 32-byte dead slot + spill round
        // trips).
        | O::VecAddF32x8
        | O::VecMulF32x8 => Some(1),
        O::VecZeroF64x4
        | O::VecLoadF64x4
        | O::VecSubF64x4
        | O::VecDivF64x4
        | O::VecXorF64x4
        | O::VecSqrtF64x4
        | O::VecRoundF64x4
        | O::VecMaddF64x4
                | O::VecMaddF64x4Signed(..)
        | O::VecCmpF64x4
        | O::VecBlendvF64x4
        | O::VecMinF64x4
        | O::VecMaxF64x4
        | O::VecAddF64x4
        | O::VecMulF64x4 => Some(2),
        O::VecZeroF32x4
        | O::VecLoadF32x4
        | O::VecSubF32x4
        | O::VecDivF32x4
        | O::VecXorF32x4
        | O::VecSqrtF32x4
        | O::VecRoundF32x4
        | O::VecCmpF32x4
        | O::VecBlendvF32x4
        | O::VecMinF32x4
        | O::VecMaxF32x4
        | O::VecAddF32x4
        | O::VecMulF32x4
        // The 128-bit F32 fused family (BB-SLP and the `vecfma`
        // contraction): `emit_vec_fma_128` resolves homed sources in
        // place and writes only the destination home / xmm0..xmm3
        // scratch — the same contract as the F64x2 trio below. Missing
        // here, a `v16f` `a*b + c` fell to the memory path (15
        // instructions, legacy `movdqu` staging) while the F64x2 twin
        // was one instruction.
        | O::VecFmaF32x4
        | O::VecFnmaF32x4
        | O::VecFmsF32x4
        | O::VecFnmsF32x4 => Some(4),
        O::VecZeroF64x2
        | O::VecLoadF64x2
        | O::VecSubF64x2
        | O::VecDivF64x2
        | O::VecXorF64x2
        | O::VecSqrtF64x2
        | O::VecRoundF64x2
        | O::VecCmpF64x2
        | O::VecBlendvF64x2
        | O::VecMinF64x2
        | O::VecMaxF64x2
        | O::VecAddF64x2
        | O::VecMulF64x2
        | O::VecFmaF64x2
        | O::VecFnmaF64x2
        | O::VecFmsF64x2
        | O::VecFnmsF64x2 => Some(5),
        // Integer map intermediates (class numbering mirrors the
        // broadcast collector): dword lanes, AVX2 8-wide and SSE2
        // 4-wide. Conditional-map results (compare masks, blends)
        // included — their emitters resolve every vector operand
        // through the register cache.
        O::VecLoadI32x8
        | O::VecSubI32x8
        | O::VecAddI32x8
        | O::VecMulI32x8
        | O::VecAndI32x8
        | O::VecOrI32x8
        | O::VecXorI32x8
        | O::VecCmpI32x8
        | O::VecCmpI64x4
        | O::VecBlendvI32x8
        | O::VecMinI32x8
        | O::VecMaxI32x8
     // Byte-lane map intermediates (AVX2 32xI8): identical home-
        // aware emitter paths (emit_avx_binary_256, emit_int_cmp_i8,
        // emit_avx_blendv_256, shared VecStore arm via
        // vec_store_source_256).
        | O::VecLoadI8x32
        | O::VecLoadI16x16
        | O::VecSubI8x32
        | O::VecAddI8x32
        | O::VecAndI8x32
        | O::VecOrI8x32
        | O::VecXorI8x32
        | O::VecCmpI8x32
        | O::VecBlendvI8x32
        | O::VecMinU8x32
        | O::VecMaxU8x32
        | O::VecMinI8x32
        | O::VecMaxI8x32
        // Word lanes (OP-05g) share the 256-bit integer class: same
        // YMM file, same VecLoad/StoreI32x8 endpoints.
        | O::VecAddI16x16
        | O::VecSubI16x16
        | O::VecMulI16x16
        | O::VecCmpI16x16
        | O::VecMinI16x16
        | O::VecMaxI16x16
        | O::VecMinU16x16
        | O::VecMaxU16x16
        | O::VecBlendvI16x16
        | O::VecAndI16x16
        | O::VecOrI16x16
        | O::VecXorI16x16
        // BB-SLP 256-bit packed lane shifts (word/dword/qword): same
        // 256-bit integer class, same YMM file.
        | O::VecShlI16x16
        | O::VecLShrI16x16
        | O::VecAShrI16x16
        | O::VecShlI32x8
        | O::VecPackssdwI16x16
        | O::VecLShrI32x8
        | O::VecAShrI32x8
        | O::VecShlI64x4
        | O::VecLShrI64x4
        // Per-lane variable shifts: two-source `emit_avx_binary_256`
        // forms, same 256-bit integer class.
        | O::VecShlvI32x8
        | O::VecLShrvI32x8
        | O::VecAShrvI32x8
        | O::VecShlvI64x4
        | O::VecLShrvI64x4 => Some(3),
        // W5: the I64x4 family rides the SAME audited emitters as the
        // I32x8 twins above (emit_avx_binary_256 / vec_store_source_
        // 256). Without these producer entries a rotate diamond
        // (load → {shl, shr} → or) left every value slot-homed: 7
        // memory ops where 2 suffice, plus a dead frame. Deferral
        // cannot cover the shift results: the second shift is an
        // intervening Intrinsic, rejected by the window analysis —
        // homing is the only sound cover.
        | O::VecLoadI64x4
        | O::VecAddI64x4
        | O::VecSubI64x4
        | O::VecAndI64x4
        | O::VecOrI64x4
        | O::VecXorI64x4 => Some(3),
        O::VecLoadI32x4
        | O::VecSubI32x4
        | O::VecAddI32x4
        | O::VecMulI32x4
        | O::VecAndI32x4
        | O::VecOrI32x4
        | O::VecXorI32x4
        | O::VecCmpI32x4
        | O::VecCmpI64x2
        | O::VecBlendvI32x4
     | O::VecRotlI32x4
        | O::VecShufdI32x4
        | O::VecShufbI32x4
        | O::VecPackI32x4
        | O::VecAddI8x16
        | O::VecSubI8x16
        | O::VecCmpI8x16
        | O::VecMinU8x16
        | O::VecMaxU8x16
        | O::VecBlendvI8x16
        | O::VecAddI16x8
        | O::VecSubI16x8
        | O::VecMulI16x8
        | O::VecCmpI16x8
        | O::VecMinI16x8
        | O::VecMaxI16x8
     | O::VecBlendvI16x8
        // BB-SLP 128-bit packed lane shifts (word/dword/qword): same
        // 128-bit integer class, same XMM file.
        | O::VecShlI16x8
        | O::VecLShrI16x8
        | O::VecAShrI16x8
        | O::VecShlI32x4
        | O::VecLShrI32x4
        | O::VecAShrI32x4
        | O::VecShlI64x2
        | O::VecLShrI64x2
        // Per-lane variable shifts (VEX.128 three-operand form).
        | O::VecShlvI32x4
        | O::VecLShrvI32x4
        | O::VecAShrvI32x4
        | O::VecShlvI64x2
        | O::VecLShrvI64x2
        // W5: I64x2 family (emit_sse_binary_128 / vec_store_source_
        // 128, the I32x4 twins) and the SSE4.1 signed dword min/max
        // (emit_sse_binary_128 via emit_sminmax_i32x4) — same rotate-
        // diamond and dead-slot reasoning as the 256-bit arms above.
        | O::VecLoadI64x2
        | O::VecAddI64x2
        | O::VecSubI64x2
        | O::VecAndI64x2
        | O::VecOrI64x2
        | O::VecXorI64x2
        | O::VecSminI32x4
        | O::VecSmaxI32x4
        // Byte/word 128-bit loads and bitwise ops: the GNU-vector
        // `v16c`/`v8s` shapes.  Home-direct `movdqu` loads (the I32x4
        // arm's discipline) and `emit_sse_binary_128`/VEX.128 bitwise
        // forms; without these entries every `v8s` parameter round-
        // tripped through a 16-byte slot.
        | O::VecLoadI8x16
        | O::VecLoadI16x8
        | O::VecAndI8x16
        | O::VecOrI8x16
        | O::VecXorI8x16
        | O::VecAndI16x8
        | O::VecOrI16x8
        | O::VecXorI16x8
        // HALF-WIDE dword-pair endpoints (the ops between them are
        // the ordinary I32x4 intrinsics above).
        | O::VecLoadI32x4Pair
        | O::VecPackI32x4Pair => Some(6),
        _ => None,
    }
}

/// Map-loop intermediates (OP-05a and the FP min/max/select extension):
/// stream loads, packed compares, blends, min/max and arithmetic results
/// whose EVERY consumer reads them through a home-aware path.  Same
/// fail-closed discipline as `collect_x86_map_broadcast_values`: one
/// non-listed consumer (or an address-side / terminator use) strands the
/// value on the stack.  Multi-use intermediates — the clamp body's stream
/// load feeding a min and a compare — are the values this collector exists
/// for: the deferral machinery can only serve single-use windows, so
/// without a register home every one of their consumers pays a slot
/// round trip per iteration.
fn collect_x86_map_intermediate_values(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp as O;

    // Every consumer must be home-aware (see the broadcast collector).  The
    // final VecStore reads its source through vec_store_source_256/128.
    let legal_consumer = |op: &O, class: u8| -> bool {
        match class {
            1 => matches!(
                op,
                O::VecSubF32x8
                    | O::VecDivF32x8
                    | O::VecSqrtF32x8
                    | O::VecRoundF32x8
                    | O::VecMaddF32x8
                    | O::VecMaddF32x8Signed(..)
                    | O::VecCmpF32x8
                    | O::VecBlendvF32x8
                    | O::VecMinF32x8
                    | O::VecMaxF32x8
                    | O::VecStoreF32x8
                    // W5: Add/Mul/Xor ride emit_avx_binary_256 / the
                    // FP-Neg sign-mask path (vec_home_256 first) — a
                    // streamed load feeding the FP-Neg xor kept a dead
                    // 32-byte slot + frame otherwise.
                    | O::VecAddF32x8
                    | O::VecMulF32x8
                    | O::VecXorF32x8
            ),
            2 => matches!(
                op,
                O::VecSubF64x4
                    | O::VecDivF64x4
                    | O::VecSqrtF64x4
                    | O::VecRoundF64x4
                    | O::VecMaddF64x4
                    | O::VecMaddF64x4Signed(..)
                    | O::VecCmpF64x4
                    | O::VecBlendvF64x4
                    | O::VecMinF64x4
                    | O::VecMaxF64x4
                    | O::VecStoreF64x4
                    // W5: Add/Mul/Xor — emit_avx_binary_256 / sign-mask
                    // path, same audit as class 1.
                    | O::VecAddF64x4
                    | O::VecMulF64x4
                    | O::VecXorF64x4
            ),
            4 => matches!(
                op,
                O::VecSubF32x4
                    | O::VecDivF32x4
                    | O::VecSqrtF32x4
                    | O::VecRoundF32x4
                    | O::VecCmpF32x4
                    | O::VecBlendvF32x4
                    | O::VecMinF32x4
                    | O::VecMaxF32x4
                    | O::VecStoreF32x4
                    // W5: Add/Mul/Xor — emit_sse_binary_128 / sign-mask
                    // path (vec_home_128 first).
                    | O::VecAddF32x4
                    | O::VecMulF32x4
                    | O::VecXorF32x4
                    | O::VecFmaF32x4
                    | O::VecFnmaF32x4
                    | O::VecFmsF32x4
                    | O::VecFnmsF32x4
            ),
            5 => matches!(
                op,
                O::VecSubF64x2
                    | O::VecDivF64x2
                    | O::VecSqrtF64x2
                    | O::VecRoundF64x2
                    | O::VecCmpF64x2
                    | O::VecBlendvF64x2
                    | O::VecMinF64x2
                    | O::VecMaxF64x2
                    | O::VecStoreF64x2
                    // W5: Add/Mul/Xor — same audit as class 4.
                    | O::VecAddF64x2
                    | O::VecMulF64x2
                    | O::VecXorF64x2
                    | O::VecFmaF64x2
                    | O::VecFnmaF64x2
                    | O::VecFmsF64x2
                    | O::VecFnmsF64x2
            ),
            3 => matches!(
                op,
                O::VecAddI32x8
                    | O::VecMulI32x8
                    | O::VecSubI32x8
                    | O::VecAndI32x8
                    | O::VecOrI32x8
                    | O::VecXorI32x8
                    | O::VecCmpI32x8
                    | O::VecCmpI64x4
                    | O::VecBlendvI32x8
                    | O::VecMinI32x8
                    | O::VecMaxI32x8
                    | O::VecAddI8x32
                    | O::VecSubI8x32
                    | O::VecCmpI8x32
                    | O::VecMinU8x32
                    | O::VecMaxU8x32
                    | O::VecBlendvI8x32
                    | O::VecMinI8x32
                    | O::VecMaxI8x32
                    // Word lanes (OP-05g) share the 256-bit integer class:
                    // same YMM file, same VecLoad/StoreI32x8 endpoints.
                    | O::VecAddI16x16
                    | O::VecSubI16x16
                    | O::VecMulI16x16
                    | O::VecCmpI16x16
                    | O::VecMinI16x16
                    | O::VecMaxI16x16
                    | O::VecMinU16x16
                    | O::VecMaxU16x16
                    | O::VecBlendvI16x16
                    | O::VecAndI16x16
                    | O::VecOrI16x16
                    | O::VecXorI16x16
                    | O::VecStoreI32x8
                    // Byte ops NOT in the list above: the bitwise byte ops
                    // (register-file identical to the dword forms) and the
                    // 128-bit compares/mins/maxes.
                    | O::VecAndI8x32
                    | O::VecOrI8x32
                    | O::VecXorI8x32
                    | O::VecCmpI8x16
                    | O::VecBlendvI8x16
                    | O::VecMinU8x16
                    | O::VecMaxU8x16
                    | O::VecStoreI8x32
                    | O::VecStoreI32x4
                    | O::VecStoreI32x4Pair
                    // BB-SLP 256-bit packed lane shifts: the immediate
                    // forms read the homed YMM source in place.
                    | O::VecShlI16x16
                    | O::VecLShrI16x16
                    | O::VecAShrI16x16
                    | O::VecShlI32x8
                    | O::VecPackssdwI16x16
                    // The final byte-pack is a home-aware 256-bit AVX
                    // binary store; permit it to consume the register-homed
                    // word packs rather than round-tripping through slots.
                    | O::Packuswb256
                    | O::VecLShrI32x8
                    | O::VecAShrI32x8
                    | O::VecShlI64x4
                    | O::VecLShrI64x4
                    | O::VecShlvI32x8
                    | O::VecLShrvI32x8
                    | O::VecAShrvI32x8
                    | O::VecShlvI64x4
                    | O::VecLShrvI64x4
                    // W5: the I64x4 binary family + store — the exact
                    // emitters of the I32x8 twins above.
                    | O::VecAddI64x4
                    | O::VecSubI64x4
                    | O::VecAndI64x4
                    | O::VecOrI64x4
                    | O::VecXorI64x4
                    | O::VecStoreI64x4
                    // Word-lane store: the `vec_store_source_256` arm
                    // shared with I32x8/I8x32.
                    | O::VecStoreI16x16
            ),
            6 => matches!(
                op,
                O::StrictRecipMulAddPackedF64x4
                    | O::VecAddI32x4
                    | O::VecMulI32x4
                    | O::VecSubI32x4
                    | O::VecAndI32x4
                    | O::VecOrI32x4
                    | O::VecXorI32x4
                    | O::VecCmpI32x4
                    | O::VecCmpI64x2
                    | O::VecBlendvI32x4
                    | O::VecAddI8x16
                    | O::VecSubI8x16
                    | O::VecCmpI8x16
                    | O::VecMinU8x16
                    | O::VecMaxU8x16
                    | O::VecBlendvI8x16
                    | O::VecAddI16x8
                    | O::VecSubI16x8
                    | O::VecMulI16x8
                    | O::VecCmpI16x8
                    | O::VecMinI16x8
                    | O::VecMaxI16x8
                    | O::VecBlendvI16x8
                    // ARX lane ops: scratch-only emitters (xmm0/xmm1 pair,
                    // the source home is never written) — a register-homed
                    // value stays live across them.
                    | O::VecStoreI32x4
                    | O::VecRotlI32x4
                    | O::VecShufdI32x4
                    | O::VecShufbI32x4
                    | O::VecExtractLaneI32x4
                    // BB-SLP 128-bit packed lane shifts: VEX.128
                    // immediate forms read the homed XMM source in place.
                    | O::VecShlI16x8
                    | O::VecLShrI16x8
                    | O::VecAShrI16x8
                    | O::VecShlI32x4
                    | O::VecLShrI32x4
                    | O::VecAShrI32x4
                    | O::VecShlI64x2
                    | O::VecLShrI64x2
                    | O::VecShlvI32x4
                    | O::VecLShrvI32x4
                    | O::VecAShrvI32x4
                    | O::VecShlvI64x2
                    | O::VecLShrvI64x2
                    // W5: I64x2 binaries + store (emit_sse_binary_128 /
                    // vec_store_source_128 — the I32x4 twins) and the
                    // SSE4.1 signed dword min/max (emit_sse_binary_128).
                    | O::VecAddI64x2
                    | O::VecSubI64x2
                    | O::VecAndI64x2
                    | O::VecOrI64x2
                    | O::VecXorI64x2
                    | O::VecSminI32x4
                    | O::VecSmaxI32x4
                    | O::VecStoreI64x2
                    // Byte/word 128-bit bitwise ops and stores (the
                    // `vec_store_source_128` arm shared with I32x4).
                    | O::VecAndI8x16
                    | O::VecOrI8x16
                    | O::VecXorI8x16
                    | O::VecAndI16x8
                    | O::VecOrI16x8
                    | O::VecXorI16x8
                    | O::VecStoreI8x16
                    | O::VecStoreI16x8
            ),
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
                if let Some(class) = x86_map_intrinsic_class(op) {
                    candidates.insert(d.0, class);
                }
            }
        }
    }

    let web_conflict = propagate_copy_webs(func, &mut candidates);
    let mut bad = web_conflict;
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |operand| {
                let Operand::Value(value) = operand else {
                    return;
                };
                let Some(&class) = candidates.get(&value.0) else {
                    return;
                };
                let legal = match inst {
                    // A homed 256-bit (classes 1..=3) or 128-bit (4..=6)
                    // value is one register image whatever lane type
                    // produced it: a float compare mask feeding
                    // `VecAndI32x8` (`v32i m = a < b; m & x`) reads the home
                    // exactly like an integer producer would.  The width
                    // group never crosses (a YMM home is not an XMM one).
                    Instruction::Intrinsic { op, .. } => {
                        let group: &[u8] = if (1..=3).contains(&class) {
                            &[1, 2, 3]
                        } else {
                            &[4, 5, 6]
                        };
                        group.iter().any(|&c| legal_consumer(op, c))
                    }
                    // Web transport: `emit_copy_value` moves a homed vector
                    // register to register.  The web poisoning below keeps a
                    // homed source from ever meeting a slot-homed destination.
                    Instruction::Copy { dest, .. } => candidates.get(&dest.0) == Some(&class),
                    _ => false,
                };
                if !legal {
                    bad.insert(value.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                if candidates.contains_key(&v.0) {
                    // Address-side / pointer-only position (intrinsic
                    // dest_ptr, Store/Load ptr, GEP base, ...): never a
                    // packed register consumer.  Unconditional, fail-closed.
                    bad.insert(v.0);
                }
            });
        }
        // A register-valued vector return (`try_lower_vector_return_simd`)
        // is home-aware: `emit_return_impl` resolves the operand through
        // `sse_load_arg` / `avx_load_arg_to` (register home first). Every
        // other terminator position evicts.
        let return_of_vector = matches!(
            &block.terminator,
            Terminator::Return(Some(Operand::Value(_)))
        );
        for_each_operand_in_terminator(&block.terminator, |operand| {
            if let Operand::Value(value) = operand {
                if candidates.contains_key(&value.0) && !return_of_vector {
                    bad.insert(value.0);
                }
            }
        });
    }
    poison_copy_webs(func, &candidates, &mut bad);
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
fn collect_f64_values(func: &IrFunction, fp_extract_homes: bool) -> FxHashSet<u32> {
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
                        | O::FmaScalarF64Signed(..)
                        | O::FmaScalarF32Signed(..)
                        | O::RoundScalarF64(_)
                        | O::RoundScalarF32(_)
                        | O::CopysignF64
                        | O::CopysignF32
                        // Horizontal FP reductions produce scalar F32/F64
                        // SSA values in the SSE domain (store_xmm_to); admit
                        // them so Phase 3 can home the results in XMM
                        // registers instead of bouncing them through GPRs
                        // and stack slots between the combine and the
                        // remainder loop / return.
                        | O::HorizontalAddF64x4
                        | O::HorizontalAddF64x2
                        | O::VecHorizontalAddF64x4
                        | O::VecHorizontalAddF64x2
                        | O::StrictRecipMulAddF64x4
                                | O::StrictRecipMulAddPackedF64x4
                        | O::VecHorizontalAddF32x8
                        | O::VecHorizontalAddF32x4
                ) || (fp_extract_homes
                    && inst.result_type().as_ref().is_some_and(is_scalar_fp)) =>
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

/// Copy-connected scalar FP webs (follow-up #3).  After phi elimination a
/// loop-carried scalar FP accumulator is transported by Copy instructions:
///
/// ```text
/// %r = VecHorizontalAddF64x4(%acc)  // combine
/// %c = Copy(%r)                     // entry handoff
/// %n = Add(%c, %term)               // loop step
/// %c = Copy(%n)                     // latch
/// ```
///
/// Without web knowledge the XMM scan homes %r, %c and %n independently
/// (their intervals touch at the copies, so a linear scan reuses one
/// register but never the combine result's), and the combine pays a
/// store/copy move on loop entry — the p16/p23 slot bounce.  Union-find
/// the Copy edges among scan-eligible values; the scan then allocates ONE
/// merged interval per web and every member shares the register, turning
/// the copies into same-register no-ops.  Only values that will actually
/// reach the scan (f64_value_set ∩ real_use, no existing assignment) may
/// join: a member pinned to an out-of-pool register (param xmm0, scratch)
/// must not drag the web into a register the scan does not own.
fn fp_copy_web_groups(
    func: &IrFunction,
    f64_value_set: &FxHashSet<u32>,
    real_use: &FxHashSet<u32>,
    assignments: &FxHashMap<u32, PhysReg>,
    segments: &[LiveInterval],
) -> FxHashMap<u32, Vec<u32>> {
    fn find(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
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
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(s),
            } = inst
            {
                let (d, s) = (dest.0, s.0);
                if d == s {
                    continue;
                }
                if !f64_value_set.contains(&d)
                    || !f64_value_set.contains(&s)
                    || !real_use.contains(&d)
                    || !real_use.contains(&s)
                    || assignments.contains_key(&d)
                    || assignments.contains_key(&s)
                {
                    continue;
                }
                parent.entry(d).or_insert(d);
                parent.entry(s).or_insert(s);
                let (rd, rs) = (find(&mut parent, d), find(&mut parent, s));
                if rd != rs {
                    // Keep the smallest id as root for determinism.
                    let (lo, hi) = (rd.min(rs), rd.max(rs));
                    parent.insert(hi, lo);
                }
            }
        }
    }
    let mut groups: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let members: Vec<u32> = parent.keys().copied().collect();
    for v in members {
        let r = find(&mut parent, v);
        groups.entry(r).or_default().push(v);
    }
    // INTERFERENCE VALIDATION (soundness, fp_copy_web_interference): a copy
    // edge implies value equality only AT the copy — members of a web hold
    // DIFFERENT values whenever their live ranges overlap beyond a copy
    // boundary (fib swap a/b, newton prev/cur, lost_copy snapshot,
    // rot3 rotation, dot_and_sum combine reused after the remainder loop).
    // One register for overlapping members corrupts one of them.  Reject
    // any group with a pair whose hole-aware segments overlap on an
    // INTERIOR point: closed segments touching at exactly the handoff/kill
    // point (max(start) == min(end)) are the legal value-transfer shape
    // (the p16/p23 combine→carry webs) and stay merged; everything else
    // falls back to separate homes (the copies stay real moves).
    let mut seg_of: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for seg in segments {
        seg_of
            .entry(seg.value_id)
            .or_default()
            .push((seg.start, seg.end));
    }
    groups.retain(|_leader, members| {
        for i in 0..members.len() {
            // FAIL CLOSED on missing coverage. A member with no recorded
            // segment is a member whose live range this validation cannot
            // see; treating "no data" as "no interference" would merge
            // exactly the webs the check exists to reject. The scan then
            // keeps separate homes and the copies stay real moves — the
            // pre-existing, always-correct behavior.
            let Some(si) = seg_of.get(&members[i]) else {
                return false;
            };
            for j in (i + 1)..members.len() {
                let Some(sj) = seg_of.get(&members[j]) else {
                    return false;
                };
                for &(a0, a1) in si {
                    for &(b0, b1) in sj {
                        if a0.max(b0) < a1.min(b1) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    });
    for members in groups.values_mut() {
        members.sort_unstable();
    }
    groups
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
                    ty,
                    ..
                }
                | Instruction::AtomicInc {
                    ptr: Operand::Value(v),
                    ty,
                    ..
                }
                | Instruction::AtomicCmpxchg {
                    ptr: Operand::Value(v),
                    ty,
                    ..
                }
                | Instruction::AtomicLoad {
                    ptr: Operand::Value(v),
                    ty,
                    ..
                }
                | Instruction::AtomicStore {
                    ptr: Operand::Value(v),
                    ty,
                    ..
                } => {
                    // x86-64's narrow atomic emitters read the address
                    // register-aware (`atomic_address_reg`: in place from its
                    // home, else via %rax/%rcx), and their only fixed scratch
                    // is modelled (`x86_inst_fixed_scratch`; %rdx refuses any
                    // value live into the point).  Excluding the pointer used
                    // to force it through a slot or the accumulator -- a
                    // reload per access for an atomic counter in a loop.
                    // 128-bit forms stage libatomic arguments in
                    // %rdi/%rsi/%rdx/%rcx before reading the pointer, and the
                    // other backends' sequences are not audited for it: both
                    // keep the exclusion.
                    let x86_narrow = crate::common::types::target_elf_machine()
                        == crate::backend::elf::EM_X86_64
                        && !matches!(ty, IrType::I128 | IrType::U128);
                    if !x86_narrow {
                        eligible.remove(&v.0);
                    }
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
    ra_config: &RaConfig,
) -> (Vec<u32>, Vec<u32>) {
    collect_i686_scratch_hazard_points_refined(func, wide, ecx_clean_load_ptrs, None, ra_config)
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
    ra_config: &RaConfig,
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
        Some(t) => compute_i686_divrem_pairs_with_config(func, t, ra_config),
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
                // F128 identity casts expand to either the direct copy
                // (six movl staging through %eax/%ecx/%edx —
                // casts.rs cast_copy_direct_f128, which fires when the
                // source slot holds the native encoding) or the x87
                // fallback (fldt/fstpt, no GPR scratch). The RA cannot
                // see slot membership, so mark BOTH registers dirty:
                // this point may clobber them. (F128<->scalar casts
                // also fall here via the catch-all; the emitter reads
                // results from the stack scratch and stages through
                // %eax, but %edx receives the pair result of
                // F128->U64, and conservatism at a rare instruction
                // costs nothing.)
                Instruction::Cast {
                    from_ty,
                    to_ty,
                    src,
                    dest,
                    ..
                } if matches!(from_ty, IrType::F128) || matches!(to_ty, IrType::F128) => {
                    // A same-value F128->F128 cast emits nothing
                    // (cast_same_value early-return in casts.rs).
                    let same_value =
                        matches!(src, Operand::Value(v) if v.0 == dest.0) && from_ty == to_ty;
                    if same_value {
                        (true, true)
                    } else {
                        (false, false)
                    }
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
                                | IrUnaryOp::ClzNonZero
                                | IrUnaryOp::Ctz
                                | IrUnaryOp::CtzNonZero
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
                if ra_config.debug_hazards {
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
                if ra_config.debug_hazards {
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
            if ra_config.debug_hazards {
                eprintln!("[HZ] fn={} pt={} ECX-DIRTY term", func.name, point);
            }
        }
        if !t_edx_clean {
            edx.push(point);
            if ra_config.debug_hazards {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    match inst {
        Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::Memcpy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArgStruct { .. } => true,
        Instruction::AtomicLoad { ty, .. }
        | Instruction::AtomicStore { ty, .. }
        | Instruction::AtomicRmw { ty, .. }
        | Instruction::AtomicCmpxchg { ty, .. }
        | Instruction::AtomicInc { ty, .. } => matches!(ty, IrType::I128 | IrType::U128),
        _ => false,
    }
}

/// Count uses of `value` outside the single instruction `(excl_block, excl_idx)`.
/// Used to tell "the latch copy is the only reader" sources (safe to leave
/// home-less for apply_phi_coalesce_assignments to inherit the phi's register)
/// from sources with additional consumers, which must keep their own home.
fn uses_excluding(func: &IrFunction, value: u32, excl_block: usize, excl_idx: usize) -> usize {
    let mut n = 0usize;
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if bi == excl_block && ii == excl_idx {
                continue;
            }
            for_each_operand_in_instruction(inst, |op| {
                if matches!(op, Operand::Value(v) if v.0 == value) {
                    n += 1;
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                if v.0 == value {
                    n += 1;
                }
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if matches!(op, Operand::Value(v) if v.0 == value) {
                n += 1;
            }
        });
    }
    n
}

/// True iff `term` reads `value` as an operand.
fn terminator_reads_value(term: &Terminator, value: u32) -> bool {
    let mut used = false;
    for_each_operand_in_terminator(term, |op| {
        if matches!(op, Operand::Value(v) if v.0 == value) {
            used = true;
        }
    });
    used
}

/// True iff every path from `source_block` (which defines `src` right before
/// the phi copy of `dest` in `copy_block`) to `use_block` passes through no
/// block that re-defines `dest`.  The eliminated copy's own block is exempt:
/// its definition disappears with the coalescing.
///
/// Sharing the home of `src` and `dest` is destructive: the source
/// definition overwrites the phi's home.  A use of `src` reached along a
/// path that runs some OTHER definition of `dest` (a second backedge copy
/// of the same loop-carried web, e.g. a `continue` latch in a loop with two
/// backedges) would then read the clobbered home.  Uses reachable only
/// through the eliminated copy — or through re-executions of the source
/// definition itself, which refresh the home with the correct value — are
/// safe.  This is a conservative search: the `dirty` flag records that the
/// path crossed a phi re-definition since the last source definition, and
/// any undetected reachability errs on the safe side.
fn src_use_path_clear_of_dest_redefs(
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    use_block: usize,
    source_block: usize,
    copy_block: usize,
    dest: u32,
    src: u32,
) -> bool {
    let mut redef_blocks: FxHashSet<usize> = FxHashSet::default();
    for (i, b) in func.blocks.iter().enumerate() {
        if i == copy_block {
            continue;
        }
        if b.instructions
            .iter()
            .any(|inst| inst.dest().is_some_and(|d| d.0 == dest))
        {
            redef_blocks.insert(i);
        }
    }
    if redef_blocks.is_empty() {
        return true;
    }
    // A use block that itself re-defines `dest` is safe only when every
    // read of `src` there (instructions and terminator) precedes the block's
    // first re-definition: a path entering it clean then reads the intact
    // home, and the DFS below treats the block as dirtying on exit.  This
    // is the if/else latch shape `x = f(old); if (rare) x' = g(x); old = x`
    // where the rare arm reads the fresh value before its own phi copy.
    // Hidden folded reads are proven by `source_home_survives_dest_redefs_in`.
    if redef_blocks.contains(&use_block) {
        let block = &func.blocks[use_block];
        let Some(first_redef) = block
            .instructions
            .iter()
            .position(|inst| inst.dest().is_some_and(|d| d.0 == dest))
        else {
            return false;
        };
        if block.instructions[first_redef..]
            .iter()
            .any(|inst| uses_value(inst, src))
            || terminator_reads_value(&block.terminator, src)
        {
            return false;
        }
    }
    // Two-state reachability from the source definition.  `dirty` means the
    // path crossed a re-definition of the phi since the last execution of
    // the source definition; re-entering the source block re-executes the
    // def and refreshes the home (the def dominates every use in SSA, so a
    // dirty path reaching a use without a fresh def is the corruption we
    // must reject).
    let mut seen: FxHashSet<(usize, bool)> = FxHashSet::default();
    let mut stack = vec![(source_block, false)];
    while let Some((cur, dirty)) = stack.pop() {
        if !seen.insert((cur, dirty)) {
            continue;
        }
        // A use reached while dirty is a corruption site: the home was
        // re-defined since the last source definition.  Do NOT stop at a
        // clean use — the loop may re-visit the use block without a fresh
        // source definition in between (e.g. second latch re-defines the
        // phi and the header routes back to the use block), so exploration
        // must continue through it with the dirty state carried along.
        if cur == use_block && dirty {
            return false;
        }
        let dirty = dirty || (cur != source_block && redef_blocks.contains(&cur));
        for &s in succs.row(cur) {
            let s = s as usize;
            stack.push((s, if s == source_block { false } else { dirty }));
        }
    }
    true
}

/// Instruction-level check that `src`'s home is not overwritten on the way
/// to the eliminated phi copy.
///
/// `src_use_path_clear_of_dest_redefs` skips the copy block entirely, so a
/// redefinition sitting in that block never vetoes coalescing. Walk every
/// instruction between the source definition and the copy, including the
/// copy block itself, and fail closed on any dest/src redef in that window.
/// Folded (hidden) reads from `liveness.folded_read_points` are real
/// consumers of the shared home — except when the folding consumer's
/// address root is the coalesced destination itself (see
/// `folded_consumer_reads_dest_home`).
/// True iff a folded (hidden) read of `src` at `consumer` actually reads the
/// coalesced destination's value from the shared home.
///
/// Liveness attributes a folded access to every value in the address root's
/// copy chain, but the emitter reads exactly one home for the address: the
/// home of the address root. When that root is the coalesced `dest`
/// (a `Load`/`Store` through `dest` directly, or through one
/// constant-displacement `GEP`/`Add` link on `dest`), the shared home holds
/// `dest`'s current runtime value — which is precisely what the access
/// needs — no matter what the static copy attribution names. Vetoing those
/// points would reject sound coalescing (zlib-ng adler32's inner `buf += 8`:
/// every `GEP(buf, off)` load is folded-attributed to the increment `v703`
/// via the latch copy `v727 = Copy v703`, while the outer-chunk `v727`
/// redefinition dirties the window on the re-entry path).
///
/// Any other shape (a folded base/index that is not syntactically `dest`,
/// a variable index, a deeper chain, a non-memory consumer) fails closed:
/// the emitter may read the shared home expecting `src`'s value while the
/// home holds `dest`'s newer value. Whether the link is actually folded by
/// the backend is irrelevant to soundness: an unfolded link never reads the
/// shared home at all, and a folded `dest`-rooted link reads the correct
/// value, so skipping the veto is sound in both cases.
fn folded_consumer_reads_dest_home(
    consumer: &Instruction,
    dest: u32,
    fold_base_of: &FxHashMap<u32, u32>,
) -> bool {
    let ptr = match consumer {
        Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => ptr.0,
        _ => return false,
    };
    if ptr == dest {
        return true;
    }
    fold_base_of.get(&ptr).copied() == Some(dest)
}

/// Constant-displacement address links (`GEP(base, const)` and integer
/// `base + const`), dest value id -> base value id. Mirrors the syntactic
/// shape liveness treats as foldable, without its foldability fixed point
/// (unfolded links make the exemption harmless, never unsound).
fn const_address_link_bases(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut bases: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(_),
                    ..
                } => {
                    bases.insert(dest.0, base.0);
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(_),
                    ty,
                }
                | Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Const(_),
                    rhs: Operand::Value(base),
                    ty,
                } if !ty.is_float() && !ty.is_long_double() => {
                    bases.insert(dest.0, base.0);
                }
                _ => {}
            }
        }
    }
    bases
}

/// Potential simultaneous-sharing components: a deliberate SUPERSET of every
/// mechanism that can place two values in one home with overlapping ranges.
///
/// Edges (unioned):
/// * multi-def-dest + single-def-src `Copy D <- S` — every phi-coalesce
///   candidate edge (multi-def srcs are never candidates);
/// * phi-transport `Copy D <- S` (`D` feeds a Phi / `S` is a Phi result);
/// * adjacent `(def(S), Copy D <- S)` — the latch same-value relaxation
///   mirrored exactly (see below);
/// * int-class `Cast D <- S` (float/decimal casts never share a GPR home).
///
/// General Copy merges and phi-congruence merges are overlap-checked
/// (time-share only): two values covering one program point cannot
/// time-share there, so they need no edge. Actual coalesced webs are
/// subsets of these components, hence values in different components
/// NEVER share a home — the soundness anchor for exempting hidden
/// folded reads whose consumer provably lives in another web (lz4: the
/// v403-rooted copy Loads extend v224/v212 through the v403<-v426<-v404
/// snapshot chain and veto the {v404,v212,v224} web even though
/// {v403,v426} provably occupies a different home).
///
/// Roots compare with [`web_find`].
fn coalesce_web_parent(func: &IrFunction) -> FxHashMap<u32, u32> {
    web_parent_with(func, true)
}

/// The components of [`coalesce_web_parent`] without its int-`Cast` edges:
/// exactly the values the phi apply phase may place in one home WITHOUT its
/// segment home-conflict test (applied pairs and their chains form its
/// same-home class).  A cast-coalesced co-resident is an ordinary holder in
/// `assignments`, which that test rejects whenever its segment starts while
/// the source is still live — so only pair-linked values need the dirty
/// tracking of [`source_home_survives_dest_redefs_in`], and vetoing on a
/// cast (`v19 = trunc v17; v40 = v17` in gzip_crc32) cost a coalescing.
fn phi_pair_web_parent(func: &IrFunction) -> FxHashMap<u32, u32> {
    web_parent_with(func, false)
}

fn web_parent_with(func: &IrFunction, casts: bool) -> FxHashMap<u32, u32> {
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut copy_dests: FxHashSet<u32> = FxHashSet::default();
    let mut phi_dests: FxHashSet<u32> = FxHashSet::default();
    let mut phi_operands: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
            }
            match inst {
                Instruction::Copy { dest, .. } => {
                    copy_dests.insert(dest.0);
                }
                Instruction::Phi { dest, incoming, .. } => {
                    phi_dests.insert(dest.0);
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            phi_operands.insert(v.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|u| {
                *use_count.entry(u).or_insert(0) += 1;
            });
        }
        block
            .terminator
            .for_each_used_value(|u| *use_count.entry(u).or_insert(0) += 1);
    }
    fn union_sets(parent: &mut FxHashMap<u32, u32>, a: u32, b: u32) {
        if a == b {
            return;
        }
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let ra = web_find(parent, a);
        let rb = web_find(parent, b);
        if ra != rb {
            parent.insert(rb, ra);
        }
    }
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    let int_class = |t: &IrType| {
        matches!(
            t,
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
                | IrType::Ptr
        )
    };
    for block in &func.blocks {
        for w in block.instructions.windows(2) {
            if let (
                prev,
                Instruction::Copy {
                    dest,
                    src: Operand::Value(s),
                },
            ) = (&w[0], &w[1])
            {
                // Mirror latch_same_value in build_coalesce_groups exactly
                // (keep in sync: loosening there without updating here misses
                // simultaneous sharings and is unsound; use_count==1 is
                // load-bearing — the zstd HUF DTable miscompile).
                if copy_dests.contains(&dest.0)
                    && def_count.get(&dest.0).copied().unwrap_or(0) > 1
                    && prev.dest().is_some_and(|pd| pd.0 == s.0)
                    && use_count.get(&s.0).copied().unwrap_or(0) == 1
                {
                    union_sets(&mut parent, dest.0, s.0);
                }
            }
        }
        for inst in &block.instructions {
            match inst {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(s),
                } => {
                    let (d, s) = (dest.0, s.0);
                    // Phi-pair arm mirrors the candidate enumeration filter
                    // (multi-def Copy dest + single-def src; keep in sync —
                    // admitting multi-def srcs there without updating here is
                    // unsound). Transport arms stay unfiltered (superset).
                    let phi_edge = copy_dests.contains(&d)
                        && def_count.get(&d).copied().unwrap_or(0) > 1
                        && def_count.get(&s).copied().unwrap_or(0) == 1;
                    let transport_edge = phi_operands.contains(&d) || phi_dests.contains(&s);
                    if phi_edge || transport_edge {
                        union_sets(&mut parent, d, s);
                    }
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(s),
                    from_ty,
                    to_ty,
                } => {
                    if casts && int_class(from_ty) && int_class(to_ty) {
                        union_sets(&mut parent, dest.0, s.0);
                    }
                }
                _ => {}
            }
        }
    }
    parent
}

/// Union-find root with path compression. Total: values without an entry
/// are singleton roots (they never share a home simultaneously — every
/// simultaneous-sharing mechanism above needs an edge).
fn web_find(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
    let mut root = x;
    loop {
        match parent.get(&root).copied() {
            None => break,
            Some(p) if p == root => break,
            Some(p) => root = p,
        }
    }
    let mut c = x;
    while c != root {
        let next = parent.get(&c).copied().unwrap_or(root);
        parent.insert(c, root);
        c = next;
    }
    root
}

/// Every value on a folded consumer's address path: the ptr itself plus
/// everything reachable by peeling what the backend can fold into the
/// addressing mode (GEP base+index, Cast, const-operand Add/Sub/Shl/Mul).
/// A SUPERSET of the backend peel set (no addr-fed filter): peeling too
/// far only costs exemptions, while stopping early would compare a folded
/// (homeless) intermediate. Multi-def values, params and unrecognized defs
/// stop the peel (they hold their own home, which the consumer reads).
/// `None` on runaway (fail-closed: no exemption).
fn consumer_address_path(
    ptr: u32,
    def_of: &FxHashMap<u32, &Instruction>,
    def_count: &FxHashMap<u32, u32>,
) -> Option<Vec<u32>> {
    let mut path = Vec::new();
    let mut stack = vec![ptr];
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    while let Some(v) = stack.pop() {
        if !seen.insert(v) {
            continue;
        }
        if seen.len() > 1024 {
            return None;
        }
        path.push(v);
        if def_count.get(&v).copied().unwrap_or(0) != 1 {
            continue;
        }
        let Some(def) = def_of.get(&v) else {
            continue;
        };
        match *def {
            Instruction::GetElementPtr { base, offset, .. } => {
                stack.push(base.0);
                if let Operand::Value(o) = offset {
                    stack.push(o.0);
                }
            }
            Instruction::Cast {
                src: Operand::Value(s),
                ..
            } => {
                stack.push(s.0);
            }
            Instruction::BinOp { op, lhs, rhs, .. }
                if matches!(
                    op,
                    IrBinOp::Add | IrBinOp::Sub | IrBinOp::Shl | IrBinOp::Mul
                ) =>
            {
                let mut has_const = false;
                let mut vals = Vec::new();
                for opnd in [lhs, rhs] {
                    match opnd {
                        Operand::Const(_) => has_const = true,
                        Operand::Value(w) => vals.push(w.0),
                    }
                }
                if has_const {
                    stack.extend(vals);
                }
            }
            _ => {}
        }
    }
    Some(path)
}

/// True when a hidden (folded-chain) read of the candidate source at
/// `consumer` provably observes a different home: every value on the
/// consumer's address path lives in a different potential-sharing
/// component than the source, so the consumer's home cannot be the
/// candidate's shared home and a dest redef cannot corrupt it.
/// Fail-closed (`false`) for non-Load/Store consumers and runaway peels.
fn hidden_read_in_disjoint_web(
    consumer: &Instruction,
    src_web: u32,
    web_parent: &mut FxHashMap<u32, u32>,
    def_of: &FxHashMap<u32, &Instruction>,
    def_count: &FxHashMap<u32, u32>,
) -> bool {
    let ptr = match consumer {
        Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => ptr.0,
        _ => return false,
    };
    let Some(path) = consumer_address_path(ptr, def_of, def_count) else {
        return false;
    };
    // Read-home distance gate (profitability, not soundness — disjointness
    // below is the soundness proof): exempt only multi-peel chains (the
    // lz4 snapshot-chain pathology: copy-Load isolation through 2+ folds,
    // read-home distance >= 2). Single-peel hidden reads (ptr -> base, the
    // consumer adjacent to the candidate) stay vetoed (legacy): merging on
    // weak evidence churns global coloring for one Copy (expat's len-2
    // {v185,v20} cost 8% with identical spill counts).
    if path.len() < 3 {
        return false;
    }
    path.iter().all(|&v| web_find(web_parent, v) != src_web)
}

/// Profitability gate for homeless-merging (one home for the union):
/// merging saves the segment overlap (two homes -> one over the shared
/// points) but costs the incremental fat-fill (union points neither
/// member covers that the merged fat range then occupies). Refuse when
/// the incremental fill exceeds the overlap — pure-gap merges (a
/// disjoint init folded into a global web) burn a global home to delete
/// one cold Copy (expat's {v185,v20} spilled where the base allocator
/// spilled nothing). Fail-open (merge) when coverage data is missing
/// (matches legacy behavior; the gate is profitability, not safety).
fn homeless_merge_profitable(segments: &[LiveInterval], dest: u32, src: u32) -> bool {
    let mut l_segs: Vec<(u32, u32)> = Vec::new();
    let mut s_segs: Vec<(u32, u32)> = Vec::new();
    for iv in segments {
        if iv.value_id == dest {
            l_segs.push((iv.start, iv.end));
        } else if iv.value_id == src {
            s_segs.push((iv.start, iv.end));
        }
    }
    if l_segs.is_empty() || s_segs.is_empty() {
        return true;
    }
    let seg_len = |segs: &[(u32, u32)]| -> u64 {
        let mut v = segs.to_vec();
        v.sort_unstable();
        let mut len: u64 = 0;
        let mut cur_s = v[0].0;
        let mut cur_e = v[0].1;
        for &(s, e) in &v[1..] {
            if s <= cur_e {
                cur_e = cur_e.max(e);
            } else {
                len += u64::from(cur_e.saturating_sub(cur_s));
                cur_s = s;
                cur_e = e;
            }
        }
        len + u64::from(cur_e.saturating_sub(cur_s))
    };
    let fat_span = |segs: &[(u32, u32)]| -> u64 {
        let lo = segs.iter().map(|&(s, _)| s).min().unwrap_or(0);
        let hi = segs.iter().map(|&(_, e)| e).max().unwrap_or(0);
        u64::from(hi.saturating_sub(lo))
    };
    let mut overlap: u64 = 0;
    for &(ls, le) in &l_segs {
        for &(ss, se) in &s_segs {
            let lo = ls.max(ss);
            let hi = le.min(se);
            if hi > lo {
                overlap += u64::from(hi - lo);
            }
        }
    }
    let mut both = l_segs.clone();
    both.extend_from_slice(&s_segs);
    let leader_fill = fat_span(&l_segs).saturating_sub(seg_len(&l_segs));
    let union_fill = fat_span(&both).saturating_sub(seg_len(&both));
    let incr_fill = union_fill.saturating_sub(leader_fill);
    overlap >= incr_fill
}

/// Function-wide facts [`source_home_survives_dest_redefs_in`] consults.
/// Each is a pure function of `func`, and both callers hold `func` immutably
/// across their whole candidate loop, so one build serves every candidate
/// (rebuilding the four O(func) maps per candidate was ~0.94 G Ir of a
/// 19.2 G Ir compile of zstd's `huf_compress.c`). `web_find` path-compresses
/// `web_parent` in place, which moves no root, so sharing it changes no answer.
struct SourceHomeFacts<'f> {
    fold_base_of: FxHashMap<u32, u32>,
    web_parent: FxHashMap<u32, u32>,
    /// [`phi_pair_web_parent`]: the dirty-tracking components.
    pair_parent: FxHashMap<u32, u32>,
    def_of: FxHashMap<u32, &'f Instruction>,
    def_count: FxHashMap<u32, u32>,
}

impl<'f> SourceHomeFacts<'f> {
    fn new(func: &'f IrFunction) -> Self {
        let mut def_of: FxHashMap<u32, &'f Instruction> = FxHashMap::default();
        let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    *def_count.entry(dest.0).or_insert(0) += 1;
                    def_of.entry(dest.0).or_insert(inst);
                }
            }
        }
        Self {
            fold_base_of: const_address_link_bases(func),
            web_parent: coalesce_web_parent(func),
            pair_parent: phi_pair_web_parent(func),
            def_of,
            def_count,
        }
    }
}

/// [`source_home_survives_dest_redefs_in`] with facts built for this one query.
#[cfg(test)]
fn source_home_survives_dest_redefs(
    func: &IrFunction,
    liveness: &LivenessResult,
    succs: &analysis::FlatAdj,
    candidate: &PhiCoalesceCandidate,
) -> bool {
    source_home_survives_dest_redefs_in(
        func,
        liveness,
        succs,
        candidate,
        &mut SourceHomeFacts::new(func),
    )
}

fn source_home_survives_dest_redefs_in(
    func: &IrFunction,
    liveness: &LivenessResult,
    succs: &analysis::FlatAdj,
    candidate: &PhiCoalesceCandidate,
    facts: &mut SourceHomeFacts<'_>,
) -> bool {
    let src = candidate.backedge_src;
    let dest = candidate.phi_dest;
    if src == dest {
        return false;
    }
    let Some(source_block) = func.blocks.get(candidate.source_block_idx) else {
        return false;
    };
    let Some(copy_block) = func.blocks.get(candidate.block_idx) else {
        return false;
    };
    if candidate.source_def_idx >= source_block.instructions.len()
        || candidate.copy_idx >= copy_block.instructions.len()
        || source_block.instructions[candidate.source_def_idx]
            .dest()
            .is_none_or(|defined| defined.0 != src)
        || !matches!(
            &copy_block.instructions[candidate.copy_idx],
            Instruction::Copy {
                dest: copy_dest,
                src: Operand::Value(copy_src),
            } if copy_dest.0 == dest && copy_src.0 == src
        )
    {
        return false;
    }

    // Instruction-level CFG walk: after dest is redefined, any read of `src`
    // (IR-visible or folded-hidden) would read a clobbered home — unless the
    // folded consumer's address root is `dest` itself, in which case the
    // shared home holds exactly the value the access needs. Reads precede
    // the definition of the same instruction, so a dirty read is checked
    // before a later write in that instruction can rescue it.
    let hidden_use_at = |point: u32| {
        liveness
            .folded_read_points
            .get(&src)
            .is_some_and(|points| points.contains(&point))
    };
    let terminator_uses_src = |terminator: &Terminator| {
        let mut used = false;
        for_each_operand_in_terminator(terminator, |operand| {
            if matches!(operand, Operand::Value(value) if value.0 == src) {
                used = true;
            }
        });
        used
    };

    // Potential-sharing components + def maps for the disjoint-web hidden-read
    // exemption below come from the caller's shared `SourceHomeFacts`.
    let src_web = web_find(&mut facts.web_parent, src);
    let dest_pair_web = web_find(&mut facts.pair_parent, dest);
    let mut work = vec![(
        candidate.source_block_idx,
        candidate.source_def_idx.saturating_add(1),
        false,
    )];
    let mut seen: FxHashSet<(usize, usize, bool)> = FxHashSet::default();
    while let Some((block_index, first_instruction, mut dirty)) = work.pop() {
        if !seen.insert((block_index, first_instruction, dirty)) {
            continue;
        }
        let Some(block) = func.blocks.get(block_index) else {
            return false;
        };
        let Some(&block_start) = liveness.block_starts.get(block_index) else {
            return false;
        };
        if first_instruction > block.instructions.len() {
            return false;
        }
        for (instruction_index, inst) in block
            .instructions
            .iter()
            .enumerate()
            .skip(first_instruction)
        {
            let Ok(offset) = u32::try_from(instruction_index) else {
                return false;
            };
            let Some(point) = block_start.checked_add(offset) else {
                return false;
            };
            if dirty && uses_value(inst, src) {
                return false;
            }
            if dirty
                && hidden_use_at(point)
                && !folded_consumer_reads_dest_home(inst, dest, &facts.fold_base_of)
                && !hidden_read_in_disjoint_web(
                    inst,
                    src_web,
                    &mut facts.web_parent,
                    &facts.def_of,
                    &facts.def_count,
                )
            {
                return false;
            }
            let Some(defined) = inst.dest() else {
                continue;
            };
            if defined.0 == src {
                // Any definition of `src` writes the shared home with the
                // current source value. Phi-elim reuses the id across
                // latches; treating extra defs as corruption rejected
                // in-place `buf += 8` on zlib-ng adler32 (stackmem 11→12).
                dirty = false;
            } else if defined.0 == dest {
                let selected =
                    block_index == candidate.block_idx && instruction_index == candidate.copy_idx;
                if !selected {
                    dirty = true;
                }
            } else if web_find(&mut facts.pair_parent, defined.0) == dest_pair_web {
                // Another pair-linked value (a sibling latch source coalesced
                // into the same home: `x' = g(x)` before its own copy
                // `old = x'`) writes the home at ITS definition, not at the
                // eliminated copy, and the apply phase exempts it from the
                // home-conflict test; every potential sibling dirties.
                dirty = true;
            }
        }
        let Ok(terminator_offset) = u32::try_from(block.instructions.len()) else {
            return false;
        };
        let Some(terminator_point) = block_start.checked_add(terminator_offset) else {
            return false;
        };
        if dirty && (terminator_uses_src(&block.terminator) || hidden_use_at(terminator_point)) {
            return false;
        }
        for &successor in succs.row(block_index) {
            work.push((successor as usize, 0, dirty));
        }
    }
    true
}

fn terminator_targets_label(term: &Terminator, label: crate::ir::reexports::BlockId) -> bool {
    match term {
        Terminator::Branch(target) => *target == label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == label || *false_label == label,
        Terminator::Switch { cases, default, .. } => {
            *default == label || cases.iter().any(|&(_, t)| t == label)
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.contains(&label),
        Terminator::Return(_) | Terminator::Unreachable => false,
    }
}

/// Longest single-predecessor chain (in intermediate blocks) that the phi
/// coalescer will reason through. Both the liveness gate and the
/// caller-saved-clobber window use this one bound: a longer reach on one
/// side would let a window be proven safe over blocks the other side never
/// looked at.
const PHI_CHAIN_MAX_MIDS: usize = 4;

/// The blocks strictly between `copy_block` and `source_block` on a
/// single-predecessor chain, ordered from `source_block` to `copy_block`.
///
/// This is the ONE definition of the chain shape. It is deliberately pure
/// shape analysis: whether a chain is *admissible* (loop depth, liveness,
/// uses) is the caller's policy, so the liveness gate and the clobber
/// window cannot drift apart and disagree about which blocks are in scope.
///
/// Returns `None` for the same-block case, for any block on the walk with
/// more than one predecessor, when the walk revisits a block (multi-block
/// cycle), and beyond [`PHI_CHAIN_MAX_MIDS`] hops.
fn single_pred_chain(
    preds: &analysis::FlatAdj,
    source_block: usize,
    copy_block: usize,
) -> Option<Vec<usize>> {
    if source_block == copy_block {
        return None;
    }
    let mut mids = Vec::new();
    let mut cur = copy_block;
    while preds.len(cur) == 1 && mids.len() <= PHI_CHAIN_MAX_MIDS {
        let p = preds.row(cur)[0] as usize;
        if p == source_block {
            mids.reverse();
            return Some(mids);
        }
        if p == copy_block || mids.contains(&p) {
            return None;
        }
        mids.push(p);
        cur = p;
    }
    None
}

/// [`single_pred_chain`] plus the loop-depth admissibility policy: every
/// intermediate block must sit at least as deep as the copy block, so the
/// source is born on the same loop-carried path rather than in a preheader.
fn single_pred_chain_blocks(
    preds: &analysis::FlatAdj,
    liveness: &LivenessResult,
    source_block: usize,
    copy_block: usize,
) -> Option<Vec<usize>> {
    let mids = single_pred_chain(preds, source_block, copy_block)?;
    let min_depth = liveness
        .block_loop_depth
        .get(copy_block)
        .copied()
        .unwrap_or(0);
    if mids
        .iter()
        .any(|&m| liveness.block_loop_depth.get(m).copied().unwrap_or(0) < min_depth)
    {
        return None;
    }
    Some(mids)
}

/// True iff a caller-saved instruction can execute between the source's
/// definition and the latch copy on *every* path the coalescer reasons
/// about: the source block's tail, the sole single-predecessor chain of
/// intermediate blocks (`single_pred_chain`, the same shape the liveness
/// gate admits), and the copy block's prefix.
///
/// `preds` is the CFG the caller already has. Rebuilding it per candidate
/// here made this check O(candidates x (V+E)) on every function's register
/// allocation; the caller now builds one CFG for the whole candidate loop.
fn phi_window_clobbers_caller_saved(
    func: &IrFunction,
    cand: &PhiCoalesceCandidate,
    preds: &analysis::FlatAdj,
) -> bool {
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
        if src_block.instructions[cand.source_def_idx + 1..]
            .iter()
            .chain(copy_block.instructions[..cand.copy_idx].iter())
            .any(instruction_clobbers_caller_saved)
        {
            return true;
        }
        // The direct-edge case needs no walk: the source tail above already
        // covers every instruction on that path. When the edge is absent,
        // every intermediate block of the chain is in the window.
        if !terminator_targets_label(&src_block.terminator, copy_block.label)
            && let Some(mids) = single_pred_chain(preds, cand.source_block_idx, cand.block_idx)
        {
            return mids.iter().any(|&m| {
                func.blocks[m]
                    .instructions
                    .iter()
                    .any(instruction_clobbers_caller_saved)
            });
        }
        false
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
) -> Vec<PhiCoalesceCandidate> {
    apply_phi_coalesce_assignments_with_config(
        func,
        liveness,
        iv_map,
        candidates,
        assignments,
        callee_saved,
        None,
        &RaConfig::default(),
    )
}

/// The values holding `reg` that block homing the back-edge source `src`
/// (live over `src_iv`) in it, sorted and deduplicated.
///
/// A holder blocks only where its SEGMENTS meet the source's: a
/// segment-filled co-tenant living around the loop (zlib_ng_adler32's
/// outer-loop v789 [68,152], whose hole spans the inner loop the phi lives
/// in) is no conflict. Fat overlap stays the cheap prefilter. Same soundness
/// basis as `fp_phi_move_conflicts`: segments cover every live point and the
/// post-RA repair reasons about exactly them; a value with no segment data
/// falls back to its fat interval. `seg_cov` is built on first use (no web
/// map: each holder's OWN pieces, as in the FP phi move).
fn phi_home_blockers(
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    assignments: &FxHashMap<u32, PhysReg>,
    same_home: &FxHashSet<u32>,
    reg: PhysReg,
    (src, src_iv): (u32, (u32, u32)),
    seg_cov: &mut Option<FxHashMap<u32, Vec<(u32, u32)>>>,
) -> Vec<u32> {
    let mut blockers: Vec<u32> = liveness
        .intervals
        .iter()
        .filter(|iv| {
            !same_home.contains(&iv.value_id)
                && assignments.get(&iv.value_id).is_some_and(|other_reg| {
                    other_reg.0 == reg.0 && intervals_overlap((iv.start, iv.end), src_iv)
                })
        })
        .map(|iv| iv.value_id)
        .collect();
    blockers.sort_unstable();
    blockers.dedup();
    if blockers.is_empty() {
        return blockers;
    }
    let cov = seg_cov.get_or_insert_with(|| owned_live_segments(liveness, &FxHashMap::default()));
    let src_cov: &[(u32, u32)] = cov
        .get(&src)
        .map_or(std::slice::from_ref(&src_iv), Vec::as_slice);
    blockers.retain(|b| {
        let fat = iv_map.get(b).copied().unwrap_or((0, u32::MAX));
        let held: &[(u32, u32)] = cov.get(b).map_or(std::slice::from_ref(&fat), Vec::as_slice);
        src_cov.is_empty()
            || held
                .iter()
                .any(|&h| src_cov.iter().any(|&s| intervals_overlap(h, s)))
    });
    blockers
}

fn apply_phi_coalesce_assignments_with_config(
    func: &IrFunction,
    liveness: &LivenessResult,
    iv_map: &FxHashMap<u32, (u32, u32)>,
    candidates: &[PhiCoalesceCandidate],
    assignments: &mut FxHashMap<u32, PhysReg>,
    callee_saved: &[PhysReg],
    rehome: Option<(&FxHashMap<u32, u32>, &[PhysReg])>,
    ra_config: &RaConfig,
) -> Vec<PhiCoalesceCandidate> {
    // Blocker re-homing (see `rehome_phi_blockers`) needs the copy-web
    // membership to keep groups intact and the body-safe caller-saved pool;
    // callers without them opt out.
    let rehome: Option<PhiBlockerRehome<'_>> = rehome
        .filter(|_| {
            !ra_config.no_phi_blocker_rehome
                && crate::common::types::target_elf_machine() == crate::backend::elf::EM_X86_64
        })
        .map(|(member_of, caller_saved)| PhiBlockerRehome {
            groups: member_of.iter().flat_map(|(&a, &b)| [a, b]).collect(),
            callee_saved,
            caller_saved,
            call_points: &liveness.call_points,
        });
    // Applied destructive updates. The post-RA repair and the verifier must
    // union exactly this set: candidates are proofs of eligibility, but only
    // applied pairs share a register at repair time. Unioning rejected
    // candidates would bless overlaps that no coalescing justifies.
    let mut applied: Vec<PhiCoalesceCandidate> = Vec::new();
    // Lazily-computed %rdx clobber points for the Phase-2x64 propagation
    // guard below (None until a %rdx-homed phi dest is actually considered).
    let mut rdx_clobbers: Option<Vec<u32>> = None;
    // Built at the first candidate that reaches the source-home walk.
    let mut source_home_facts: Option<SourceHomeFacts<'_>> = None;
    // Lazily-built CFG for this function: predecessor sets feed the
    // caller-saved window check, successor sets feed the source-home
    // redefinition proof. One build per function -- deriving either inside
    // the candidate loop made both checks O(candidates x (V+E)).
    let mut cfg: Option<(analysis::FlatAdj, analysis::FlatAdj)> = None;
    // Per-value segment coverage for the home-conflict test, built on first
    // use (no web map: each holder's OWN pieces, as in the FP phi move).
    let mut seg_cov: Option<FxHashMap<u32, Vec<(u32, u32)>>> = None;
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

        let succs = &cfg
            .get_or_insert_with(|| {
                let label_to_idx = analysis::build_label_map(func);
                analysis::build_cfg(func, &label_to_idx)
            })
            .1;
        if !source_home_survives_dest_redefs_in(
            func,
            liveness,
            succs,
            candidate,
            source_home_facts.get_or_insert_with(|| SourceHomeFacts::new(func)),
        ) {
            if ra_config.debug_phi_coalesce {
                eprintln!(
                    "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{}: source home does not survive dest redefs",
                    phi_dest, backedge_src
                );
            }
            continue;
        }

        let Some(&reg) = assignments.get(&phi_dest) else {
            continue;
        };

        let preds = &cfg
            .get_or_insert_with(|| {
                let label_to_idx = analysis::build_label_map(func);
                analysis::build_cfg(func, &label_to_idx)
            })
            .0;
        if phi_window_clobbers_caller_saved(func, candidate, preds)
            && !callee_saved.iter().any(|r| r.0 == reg.0)
        {
            if ra_config.debug_phi_coalesce {
                eprintln!(
                    "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{} r{}: call in window",
                    phi_dest, backedge_src, reg.0
                );
            }
            continue;
        }

        // A source whose live range spans a call point cannot inherit a
        // caller-saved home even when the (def, copy) window is call-free:
        // the uses AFTER the window reload from a register the call
        // clobbered.  Reproduced by loop_rotate_seq_loops: the rotated
        // second loop's `s` (backedge source) was re-homed into the phi's
        // caller-saved register and the post-loop printf + `s != 36` Cmp
        // read the callee's leftovers.  The value's own allocation path
        // models the call (callee-saved home or slot save/reload around
        // the call point); keep that home instead of overwriting it.
        if let Some(&src_iv) = iv_map.get(&backedge_src) {
            let src_live = LiveInterval {
                start: src_iv.0,
                end: src_iv.1,
                value_id: backedge_src,
            };
            if spans_any_call(&src_live, &liveness.call_points)
                && !callee_saved.iter().any(|r| r.0 == reg.0)
            {
                if ra_config.debug_phi_coalesce {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{} r{}: source live across a call",
                        phi_dest, backedge_src, reg.0
                    );
                }
                continue;
            }
        }

        // Phase-2x64 guard: the dest's home may be %rdx under the
        // position-aware admission wave, whose hazard filtering the SOURCE
        // value never passed. Propagating that home to a backedge source
        // whose interval crosses an %rdx clobber point would hand the
        // clobbering instruction (cqo, jump-table dispatch, atomic staging)
        // a live victim — the exact failure the wave's own candidate filter
        // exists to prevent for the dest. Cheap for every other shape: the
        // point list is computed at most once and only when a %rdx home is
        // actually in play (a clobber-free body yields an empty list, so
        // clean %rdx homes propagate exactly as before).
        if reg.0 == 16
            && crate::common::types::target_elf_machine() == crate::backend::elf::EM_X86_64
        {
            let rdx_clobbers =
                rdx_clobbers.get_or_insert_with(|| collect_x64_rdx_clobber_points(func));
            if let Some(&src_iv) = iv_map.get(&backedge_src) {
                let src_live = LiveInterval {
                    start: src_iv.0,
                    end: src_iv.1,
                    value_id: backedge_src,
                };
                if overlaps_inclusive_skip_birth(&src_live, rdx_clobbers) {
                    if ra_config.debug_phi_coalesce {
                        eprintln!(
                            "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{} r{}: source live across an rdx clobber point",
                            phi_dest, backedge_src, reg.0
                        );
                    }
                    continue;
                }
            }
        }

        if let Some(&src_iv) = iv_map.get(&backedge_src) {
            // Same-home class: the values proven to share this home by the
            // pairs applied so far, plus this candidate's own link. In a
            // Part-2 chain the phi dest rides with its upstream dest (v48
            // co-resides with v46 once `{v46, v48}` is applied), and
            // treating it as a conflicting holder would silently break the
            // chain link (`{v48, v42}` rejected for "conflict" with v46).
            let mut same_home: FxHashSet<u32> = FxHashSet::default();
            same_home.insert(backedge_src);
            same_home.insert(phi_dest);
            let mut stack = vec![backedge_src, phi_dest];
            while let Some(v) = stack.pop() {
                for p in &applied {
                    if p.phi_dest == v || p.backedge_src == v {
                        for &w in [p.phi_dest, p.backedge_src].iter() {
                            if same_home.insert(w) {
                                stack.push(w);
                            }
                        }
                    }
                }
            }
            let blockers = phi_home_blockers(
                liveness,
                iv_map,
                assignments,
                &same_home,
                reg,
                (backedge_src, src_iv),
                &mut seg_cov,
            );
            let has_conflict = !blockers.is_empty()
                && !rehome.as_ref().is_some_and(|ctx| {
                    rehome_phi_blockers(
                        ctx,
                        &blockers,
                        iv_map,
                        assignments,
                        &applied,
                        reg,
                        ra_config.debug_phi_coalesce.then_some(func.name.as_str()),
                    )
                });
            if has_conflict {
                if ra_config.debug_phi_coalesce {
                    let held: Vec<String> = blockers
                        .iter()
                        .map(|b| {
                            let iv = iv_map.get(b).copied().unwrap_or((0, 0));
                            format!("v{b}[{},{}]", iv.0, iv.1)
                        })
                        .collect();
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED assign dest=v{} src=v{} r{} src_iv=[{},{}]: home conflict with {}",
                        phi_dest,
                        backedge_src,
                        reg.0,
                        src_iv.0,
                        src_iv.1,
                        held.join(" ")
                    );
                }
                continue;
            }
        }
        // Bisect aid: CCC_PHI_COALESCE_SKIP=<src-id,...> vetoes the
        // destructive update for the listed backedge sources (optionally
        // scoped with CCC_PHI_COALESCE_FUNC=<function name>).
        if phi_coalesce_skip_listed_with_config(func, backedge_src, ra_config) {
            if ra_config.debug_phi_coalesce {
                eprintln!(
                    "[PHI_COALESCE] fn={} SKIPPED (env) dest=v{} src=v{} r{}",
                    func.name, phi_dest, backedge_src, reg.0
                );
            }
            continue;
        }
        if ra_config.debug_phi_coalesce {
            eprintln!(
                "[PHI_COALESCE] fn={} ASSIGN dest=v{} src=v{} r{}",
                func.name, phi_dest, backedge_src, reg.0
            );
        }
        assignments.insert(backedge_src, reg);
        applied.push(*candidate);
    }
    applied
}

/// Inputs of [`rehome_phi_blockers`].
struct PhiBlockerRehome<'a> {
    /// Every leader and member of a copy-coalesce group.
    groups: FxHashSet<u32>,
    /// The function's callee-saved pool (inline-asm clobbers filtered out,
    /// empty for functions with non-local exits).
    callee_saved: &'a [PhysReg],
    /// The body-safe caller-saved pool Phase 2 hands to any value that is
    /// not call-spanning (the prologue already dropped %rdi/%rdx when the
    /// body has fixed-scratch clobbers; a hazard-admitted %rdx is absent).
    caller_saved: &'a [PhysReg],
    call_points: &'a [u32],
}

/// PHI-COALESCE-HOME-CONFLICT: move every value that blocks a loop phi
/// coalesce (it holds the phi dest's register while the backedge source is
/// live) into another register that is free over its whole interval, so the
/// latch copy disappears from the loop. zlib_ng_adler32's checksum loop kept
/// three latch copies per iteration: one-point post-loop temporaries had
/// been handed the phi dest's home after the backedge source -- live out of
/// the loop -- was allocated elsewhere.
///
/// All-or-nothing, and every condition fails closed:
/// * a blocker in a copy-coalesce group (membership is re-propagated from
///   the leader later) or in an applied phi pair (its partner shares the
///   home) never moves;
/// * a target must be free over the blocker's whole fat interval with
///   INCLUSIVE endpoints (stricter than the allocator's death-point
///   sharing); a holder without an interval rejects the target;
/// * callee-saved targets come first, and only ones some value already
///   holds, so the move never adds a prologue save/restore pair; the pool
///   is call-safe by construction;
/// * a caller-saved target additionally needs a blocker that ALREADY holds
///   a caller-saved home (it passed Phase 2's admission: not call-spanning,
///   not an ordered/restricted parameter) and whose interval touches no
///   call point, inclusive -- argument staging and indirect-target homes
///   are constraints AT a call point, so none of Phase 2's call-argument
///   waves can apply to it.
///
/// x86-64 only (the caller gates it): the i686 callee-saved %esi/%edi have
/// no byte forms, and no other target's allocator was measured with it.
/// Returns true after committing the moves.
fn rehome_phi_blockers(
    ctx: &PhiBlockerRehome<'_>,
    blockers: &[u32],
    iv_map: &FxHashMap<u32, (u32, u32)>,
    assignments: &mut FxHashMap<u32, PhysReg>,
    applied: &[PhiCoalesceCandidate],
    phi_reg: PhysReg,
    debug_fn: Option<&str>,
) -> bool {
    let overlaps = |a: (u32, u32), b: (u32, u32)| a.0 <= b.1 && b.0 <= a.1;
    let mut moves: Vec<(u32, PhysReg)> = Vec::with_capacity(blockers.len());
    for &b in blockers {
        let pinned = ctx.groups.contains(&b)
            || applied
                .iter()
                .any(|p| p.phi_dest == b || p.backedge_src == b);
        let Some(&b_iv) = iv_map.get(&b).filter(|_| !pinned) else {
            return false;
        };
        let Some(&home) = assignments.get(&b) else {
            return false;
        };
        // (used by someone, free over b_iv). Blockers already moved to `r`
        // count as users but never as conflicts: every blocker co-resides in
        // the phi's register today, so any two of them are proven compatible
        // there (a two-address chain touching at its death points, or
        // segment-disjoint fills) and keep exactly that relation together
        // in `r`. Judging them inclusively split adler32's post-loop
        // `v816[127,128] v817[128,129] v819[129,130]` chain into three
        // registers and the move failed.
        let occupancy = |r: PhysReg, moves: &[(u32, PhysReg)]| {
            let mut used = moves.iter().any(|&(_, h)| h.0 == r.0);
            for (&v, &h) in assignments.iter() {
                if h.0 != r.0 || v == b {
                    continue;
                }
                used = true;
                if iv_map.get(&v).is_none_or(|&iv| overlaps(iv, b_iv)) {
                    return (true, false);
                }
            }
            (used, true)
        };
        let callee_target =
            ctx.callee_saved.iter().copied().find(|&r| {
                r.0 != phi_reg.0 && r.0 != home.0 && occupancy(r, &moves) == (true, true)
            });
        let caller_ok = ctx.caller_saved.iter().any(|r| r.0 == home.0) && {
            let idx = ctx.call_points.partition_point(|&cp| cp < b_iv.0);
            ctx.call_points.get(idx).is_none_or(|&cp| cp > b_iv.1)
        };
        let target = callee_target.or_else(|| {
            caller_ok
                .then(|| {
                    ctx.caller_saved
                        .iter()
                        .copied()
                        .find(|&r| r.0 != phi_reg.0 && r.0 != home.0 && occupancy(r, &moves).1)
                })
                .flatten()
        });
        let Some(target) = target else {
            return false;
        };
        moves.push((b, target));
    }
    for &(b, target) in &moves {
        if let Some(name) = debug_fn {
            eprintln!(
                "[PHI_COALESCE] fn={} REHOME blocker v{} r{} -> r{}",
                name, b, phi_reg.0, target.0
            );
        }
        assignments.insert(b, target);
    }
    true
}

/// `CCC_PHI_COALESCE_SKIP` / `CCC_PHI_COALESCE_FUNC` bisect helper (see
/// `apply_phi_coalesce_assignments`). Off unless the variable is set.
fn phi_coalesce_skip_listed(func: &IrFunction, backedge_src: u32) -> bool {
    phi_coalesce_skip_listed_with_config(func, backedge_src, &RaConfig::default())
}

fn phi_coalesce_skip_listed_with_config(
    func: &IrFunction,
    backedge_src: u32,
    ra_config: &RaConfig,
) -> bool {
    let Some(list) = ra_config.phi_coalesce_skip.as_deref() else {
        return false;
    };
    if let Some(f) = ra_config.phi_coalesce_func.as_deref() {
        if f != func.name {
            return false;
        }
    }
    list.split(',')
        .filter_map(|t| {
            t.trim()
                .strip_prefix('v')
                .unwrap_or(t.trim())
                .parse::<u32>()
                .ok()
        })
        .any(|v| v == backedge_src)
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
/// Compatibility helper for isolated slot-layout/unit-test callers.
/// Part 2 (FOLLOWUP-2026-09-22B §4): the narrow multi-def-source exception
/// for the split-latch conditional-update shape.
///
/// For the latch copy `dest ← m` where `m` is multi-def, this returns
/// `(u_block, u_def_idx, m_def_idx)` — the single non-passthrough def
/// `u`'s site plus `m`'s own def in `u`'s block — exactly when the copy
/// block sees `m` from a known def on EVERY incoming path:
///
///   * every def of `m` is a `Copy`;
///   * `m` has exactly one non-passthrough def (`m ← u`, `u != dest`) and
///     at least one passthrough def (`m ← dest`);
///   * `u` is single-def, its block is a (different) predecessor of the
///     copy block, and it defines `m` from `u` (the copy `m ← u`);
///   * every OTHER predecessor of the copy block contains a passthrough
///     def `m ← dest` — each path without `u` still defines `m`;
///   * `u`'s block is no shallower than the copy block (the source path
///     must be loop-carried, not a preheader init — mirrors the
///     `pred_is_unique` preheader veto).
///
/// Then `dest ← m` is a no-op once `m` shares `dest`'s home, and chaining
/// `m`'s home onto `u`'s register (usually a single-def Part-1 candidate)
/// lets the update land in place. Kernel `check_cpuflags`
/// (`err |= 1 << i`, `-m16` boot build) is the canonical instance: the web
/// `{acc, phi-dest, or-result}` takes one register instead of leaving two
/// pass-through copies per iteration.
///
/// The general multi-def-source guard is NOT relaxed: anything outside
/// this exact shape returns `None` (session-59 "the exemption stays").
fn detect_part2_chain(
    func: &IrFunction,
    liveness: &LivenessResult,
    preds: &analysis::FlatAdj,
    all_defs: &FxHashMap<u32, Vec<&Instruction>>,
    unique_def_site: &FxHashMap<u32, Option<(usize, usize)>>,
    phi_dest: u32,
    m: u32,
    copy_block: usize,
) -> Option<(usize, usize, usize)> {
    let defs = all_defs.get(&m)?;
    let mut non_passthrough_srcs: Vec<u32> = Vec::new();
    let mut passthroughs = 0usize;
    for d in defs {
        // (1) every def of `m` is a Copy.
        let Instruction::Copy {
            src: Operand::Value(s),
            ..
        } = d
        else {
            return None;
        };
        if s.0 == phi_dest {
            passthroughs += 1;
        } else {
            non_passthrough_srcs.push(s.0);
        }
    }
    // (2) exactly one non-passthrough def; (3) at least one passthrough.
    if non_passthrough_srcs.len() != 1 || passthroughs == 0 {
        return None;
    }
    let u = non_passthrough_srcs[0];
    // (4) `u` is single-def (the window proof needs its exact site).
    let Some((u_block, u_def_idx)) = unique_def_site.get(&u).copied().flatten() else {
        return None;
    };
    // The branch that produces `u` always leaves its block, so the copy
    // block is distinct.
    if u_block == copy_block {
        return None;
    }
    // (5) `u`'s block is a predecessor of the copy block.
    if !preds.row(copy_block).iter().any(|&p| p as usize == u_block) {
        return None;
    }
    // (6) `u`'s block defines `m` from `u` (the copy `m ← u`), after `u`.
    let m_def_idx = func.blocks[u_block].instructions.iter().position(|inst| {
        matches!(
            inst,
            Instruction::Copy {
                dest,
                src: Operand::Value(s),
            } if dest.0 == m && s.0 == u
        )
    })?;
    if m_def_idx <= u_def_idx {
        return None;
    }
    // (7) every other predecessor of the copy block contains a
    //     passthrough def `m ← dest`.
    for &p in preds.row(copy_block) {
        let p = p as usize;
        if p == u_block {
            continue;
        }
        let has_passthrough = func.blocks[p].instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Copy {
                    dest,
                    src: Operand::Value(s),
                } if dest.0 == m && s.0 == phi_dest
            )
        });
        if !has_passthrough {
            return None;
        }
    }
    // (8) born inside the loop (mirrors the `pred_is_unique` preheader
    //     veto): the source path must be loop-carried.
    if liveness.block_loop_depth.get(u_block).copied().unwrap_or(0)
        < liveness
            .block_loop_depth
            .get(copy_block)
            .copied()
            .unwrap_or(0)
    {
        return None;
    }
    Some((u_block, u_def_idx, m_def_idx))
}

pub(crate) fn detect_phi_coalesce_groups(
    func: &IrFunction,
    liveness: &LivenessResult,
) -> Vec<PhiCoalesceCandidate> {
    detect_phi_coalesce_groups_with_config(func, liveness, &RaConfig::default())
}

pub(crate) fn detect_phi_coalesce_groups_with_config(
    func: &IrFunction,
    liveness: &LivenessResult,
    ra_config: &RaConfig,
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
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);

    // Address-side peel closure: dest values of chains the backend actually
    // folds into a dereferenced access.  A constant-operand Shl/Mul/Add/Sub
    // is only ever ABSORBED into a SIB operand when its chain FEEDS a
    // GetElementPtr that a Load/Store dereferences; a materialized binop
    // (`v3 = n1 * 2; return v3`) is emitted at its own point into its own
    // register and its later uses carry no register dependency on the phi.
    // Without this gate the peel arm vetoed every constant-operand binop
    // derived from the phi — including the adler-style materialized shape
    // (accepts_materialized_binop_escaping_source_block) — costing a copy
    // per iteration in the hottest loops.  Backward walk from every
    // Load/Store pointer through the peelable def links (Cast, Copy, GEP,
    // constant-operand Shl/Mul/Add/Sub) collects exactly the foldable
    // chain members; the syntactic veto below consults it for binops.
    // (The Cast/GEP arms keep their unconditional behavior: pointers and
    // widened indices are deferred as a whole, and upstream's accepted
    // contract and tests are built on that.)
    let mut all_defs: FxHashMap<u32, Vec<&Instruction>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                all_defs.entry(d.0).or_default().push(inst);
            }
        }
    }
    let is_peelable_def = |inst: &Instruction| -> bool {
        match inst {
            Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. } => true,
            Instruction::BinOp {
                op: IrBinOp::Shl | IrBinOp::Mul | IrBinOp::Add | IrBinOp::Sub,
                ..
            } => {
                let mut has_const = false;
                for_each_operand_in_instruction(inst, |op| {
                    if matches!(op, Operand::Const(_)) {
                        has_const = true;
                    }
                });
                has_const
            }
            _ => false,
        }
    };
    let mut addr_fed: FxHashSet<u32> = FxHashSet::default();
    {
        let mut stack: Vec<u32> = Vec::new();
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                        stack.push(ptr.0);
                    }
                    _ => {}
                }
            }
        }
        while let Some(v) = stack.pop() {
            if !addr_fed.insert(v) {
                continue;
            }
            if let Some(defs) = all_defs.get(&v) {
                for def in defs {
                    if is_peelable_def(def) {
                        for_each_operand_in_instruction(def, |op| {
                            if let Operand::Value(w) = op {
                                stack.push(w.0);
                            }
                        });
                    }
                }
            }
        }
    }

    let debug = ra_config.debug_phi_coalesce;
    let mut candidates = Vec::new();
    // Built at the first candidate that reaches the source-home walk.
    let mut source_home_facts: Option<SourceHomeFacts<'_>> = None;

    // Pre-def dataflow closure of `phi` inside `block[..def_idx]`: every value
    // computed from the OLD phi before the destructive update.  A folded GEP
    // (`p = &a[i]`) reads `i`'s register at the Load/Store that absorbs it,
    // so such a value read after the update observes the NEW `i`.
    //
    // Propagation is restricted to address-forming instructions (Cast and
    // GetElementPtr): only those are ever *deferred* by the backend — x86
    // folds GEP/cast chains into the SIB operand of the consuming
    // Load/Store, re-reading the chain's root registers at the consumer.
    // Every other instruction materializes its dest in a register at its
    // own point (before the update), so its later uses carry no register
    // dependency on the phi.  Propagating through all instructions was
    // conservative to the point of deadlocking the common split-latch
    // loop: in `do { a += buf[n]; s += a; } while (++n < len)` the byte
    // Load depends on the `n`-indexed GEP, which put `a_next`/`s_next`
    // into `n`'s derived set, and the latch copies of a/s then counted as
    // reads of a derived value — vetoing the n-chain copy forever.
    let derived_before =
        |block: &crate::ir::reexports::BasicBlock, def_idx: usize, phi: u32| -> FxHashSet<u32> {
            let mut derived: FxHashSet<u32> = FxHashSet::default();
            derived.insert(phi);
            for earlier in &block.instructions[..def_idx] {
                // Mirror the backend's SIB `resolve_index` peel set exactly:
                // widening Cast, GEP, and the constant-operand scale/offset
                // arithmetic (`Shl`/`Mul` by a constant → SIB scale,
                // `Add`/`Sub` a constant → SIB displacement). Each of these is
                // folded away when the access absorbs the chain, so the root
                // register is re-read at the Load/Store. Missing the `Shl` arm
                // let `xs[i] = 1.0/(i+1)` share `i`'s register with `i+1`: the
                // folded `(%base,%i,8)` store then indexed with the incremented
                // value (fpweb_dot_and_sum / init_shift_index regression).
                let peelable_binop = matches!(
                    earlier,
                    Instruction::BinOp {
                        op: IrBinOp::Shl | IrBinOp::Mul | IrBinOp::Add | IrBinOp::Sub,
                        ..
                    }
                ) && {
                    let mut has_const = false;
                    for_each_operand_in_instruction(earlier, |op| {
                        if matches!(op, Operand::Const(_)) {
                            has_const = true;
                        }
                    });
                    has_const
                } && earlier.dest().is_some_and(|d| addr_fed.contains(&d.0));
                if !matches!(
                    earlier,
                    Instruction::Cast { .. } | Instruction::GetElementPtr { .. }
                ) && !peelable_binop
                {
                    continue;
                }
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
            derived
        };
    let terminator_uses = |term: &Terminator, set: &FxHashSet<u32>| -> bool {
        let mut found = false;
        for_each_operand_in_terminator(term, |op| {
            if matches!(op, Operand::Value(v) if set.contains(&v.0)) {
                found = true;
            }
        });
        found
    };

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
            if !multi_def.contains(&dest.0) {
                continue;
            }
            // Part 2 (FOLLOWUP-2026-09-22B §4): the narrow multi-def-source
            // exception. The proofs below run on `u`'s site (the wider
            // window); the candidate records `m`'s def in `u`'s block so
            // the apply-phase structural checks and the slot-coalescing
            // contract see a real def of the backedge source.
            let part2 = if multi_def.contains(&src.0) {
                detect_part2_chain(
                    func,
                    liveness,
                    &preds,
                    &all_defs,
                    &unique_def_site,
                    dest.0,
                    src.0,
                    block_idx,
                )
            } else {
                None
            };
            let (source_block, source_def_idx, cand_def_idx) = if let Some((
                u_block,
                u_def_idx,
                m_def_idx,
            )) = part2
            {
                (u_block, u_def_idx, m_def_idx)
            } else {
                if multi_def.contains(&src.0) {
                    if debug {
                        eprintln!(
                            "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}): multi-def source outside the Part-2 chain shape",
                            dest.0, src.0
                        );
                    }
                    continue;
                }
                let Some(site) = unique_def_site.get(&src.0).copied().flatten() else {
                    if debug {
                        eprintln!(
                            "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}): source is not single-def",
                            dest.0, src.0
                        );
                    }
                    continue;
                };
                (site.0, site.1, site.1)
            };

            let same_block = source_block == block_idx;
            // Split-latch shape (any type): the loop body ends in a
            // conditional branch whose loop edge is critical, so phi
            // elimination parks the copies in a one-predecessor split block
            // (`do { DO8 } while (--n)`, every rotated counted loop, every
            // `while` whose latch is also an exit test).  This is the most
            // common hot-loop shape and it used to be excluded for every
            // non-FP value, leaving 2 copies per accumulator per iteration in
            // zlib-ng adler32, expat and the arith_loop webs.  The
            // destructive-update proof for it is completed below by
            // `phi_derived_used_in_window`'s cross-block arm: the old phi
            // (and anything derived from it before the update) may not be
            // observed after the source's definition along ANY path out of
            // the source block, not merely along the latch.
            let mid_blocks = if !same_block {
                single_pred_chain_blocks(&preds, liveness, source_block, block_idx)
            } else {
                None
            };
            let pred_is_unique = mid_blocks.is_some();
            let mid_blocks = mid_blocks.unwrap_or_default();
            // Part 2 proves its predecessor structure in `detect_part2_chain`
            // (`u`'s block is a pred of the copy block and every other pred
            // carries the passthrough), so it bypasses this gate; the
            // single-def path keeps it unchanged.
            let part2_pred_ok = part2.is_some();
            if !part2_pred_ok && !(same_block && source_def_idx < copy_idx || pred_is_unique) {
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
                    .chain(
                        mid_blocks
                            .iter()
                            .flat_map(|&m| func.blocks[m].instructions.iter()),
                    )
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
                let derived = derived_before(block, source_def_idx, dest.0);
                block.instructions[source_def_idx + 1..copy_idx]
                    .iter()
                    .any(|middle| derived.iter().any(|&v| uses_value(middle, v)))
            } else {
                // Cross-block proof.  The register is overwritten at the
                // source definition inside `source_block`; from that point on
                // the old phi value is gone on EVERY path, so it (and every
                // pre-update derived value) must be dead on every path:
                //   1. not read by the rest of the source block or its
                //      terminator (`switch (state)` after `state_next = ..`);
                //   2. not read before the copy in the split latch;
                //   3. not live into any successor of the source block other
                //      than the latch — the classic `return i` after a
                //      `for (...; ++i)` exit branch reads the OLD i;
                //   4. no derived value live out of the source block at all:
                //      a folded GEP rooted at the phi is re-formed from the
                //      phi register wherever it is consumed.
                let src_blk = &func.blocks[source_block];
                let derived = derived_before(src_blk, source_def_idx, dest.0);
                let read_in_tail = src_blk.instructions[source_def_idx + 1..]
                    .iter()
                    .chain(
                        mid_blocks
                            .iter()
                            .flat_map(|&m| func.blocks[m].instructions.iter()),
                    )
                    .chain(block.instructions[..copy_idx].iter())
                    .any(|middle| derived.iter().any(|&v| uses_value(middle, v)));
                let read_by_terminator = terminator_uses(&src_blk.terminator, &derived)
                    || mid_blocks
                        .iter()
                        .any(|&m| terminator_uses(&func.blocks[m].terminator, &derived));
                let first_hop = mid_blocks.first().copied().unwrap_or(block_idx);
                let phi_escapes =
                    succs.row(source_block).iter().any(|&s| {
                        s as usize != first_hop && liveness.is_live_in(s as usize, dest.0)
                    }) || mid_blocks.iter().enumerate().any(|(i, &m)| {
                        let next_hop = mid_blocks.get(i + 1).copied().unwrap_or(block_idx);
                        liveness.is_live_in(m, dest.0)
                            || succs.row(m).iter().any(|&s| {
                                s as usize != next_hop && liveness.is_live_in(s as usize, dest.0)
                            })
                    }) || liveness.is_live_in(block_idx, dest.0);
                let derived_escapes = derived
                    .iter()
                    .filter(|&&v| v != dest.0)
                    .any(|&v| liveness.is_live_out(source_block, v));
                read_in_tail || read_by_terminator || phi_escapes || derived_escapes
            };
            // Uses of the source in *other* blocks used to veto coalescing
            // outright: after sharing the home, the only hazard is a path
            // from the source definition to such a use that re-defines the
            // phi dest (another backedge copy of the same loop-carried web
            // would clobber the shared home before the use reads it).  A
            // direct exit successor that reads the freshly computed value
            // (`return b` after the last iteration) is safe: the def wrote
            // exactly the value the exit consumes.  Check path cleanliness
            // per use block instead of rejecting blindly.
            let source_used_elsewhere = src_use_blocks.get(&src.0).is_some_and(|blocks| {
                blocks.iter().any(|&use_block| {
                    use_block != block_idx
                        && use_block != source_block
                        && !src_use_path_clear_of_dest_redefs(
                            func,
                            &succs,
                            use_block as usize,
                            source_block,
                            block_idx,
                            dest.0,
                            src.0,
                        )
                })
            });
            // The candidate records `m`'s def in `u`'s block (not `u`'s
            // own site): the apply-phase structural checks require the
            // site to define the backedge source, and the CFG walk from
            // there sees every path that reaches the copy. The window
            // proofs above already ran on the wider `u`-site window.
            let source_home_probe = PhiCoalesceCandidate {
                phi_dest: dest.0,
                backedge_src: src.0,
                block_idx,
                source_block_idx: source_block,
                source_def_idx: cand_def_idx,
                copy_idx,
            };
            let source_home_killed = !source_home_survives_dest_redefs_in(
                func,
                liveness,
                &succs,
                &source_home_probe,
                source_home_facts.get_or_insert_with(|| SourceHomeFacts::new(func)),
            );
            // Liveness-model veto: the source's definition overwrites the
            // phi's home, so the phi's OLD value may not be read at ANY
            // program point strictly inside the update window (after the
            // source def, before the latch copy).
            // Hidden SIB reads are the RA-invisible class: the backend folds
            // GEP chains into the access's addressing and re-reads the
            // base/index register at the Load/Store with no IR operand.
            // The exact read points come from the liveness folded-index walk
            // (`folded_read_points` below).  The syntactic closure above
            // remains as a cheap first filter and as coverage for the
            // CCC_NO_FOLDED_INDEX_LIVENESS debug configuration, where the
            // extension is off.
            // Update-window points (closed ranges) between the source
            // definition and the latch copy.
            let def_point = liveness.block_starts[source_block] + source_def_idx as u32;
            let copy_point = liveness.block_starts[block_idx] + copy_idx as u32;
            let mut windows: [(u32, u32); 2] = [(1, 0), (1, 0)];
            if same_block {
                if copy_point > def_point + 1 {
                    windows[0] = (def_point + 1, copy_point - 1);
                }
            } else {
                let src_end = liveness.block_ends[source_block];
                if src_end > def_point {
                    windows[0] = (def_point + 1, src_end);
                }
                let latch_start = liveness.block_starts[block_idx];
                if copy_point > latch_start {
                    windows[1] = (latch_start, copy_point - 1);
                }
            }
            // Hidden SIB reads only: the IR-visible window uses are already
            // covered exactly by `phi_used_in_window` above.  A folded access
            // (GEP absorbed into the addressing) re-reads the base/index
            // register at its OWN point with no IR operand; the liveness walk
            // records those points exactly (`folded_read_points`), so test
            // them directly instead of the block-granular segment cover — a
            // same-block latch phi that is live-in (used early in the block)
            // AND live-out (redefined by the latch copy) gets a whole-block
            // segment that overlaps every interior window, which vetoed all
            // tight-loop accumulator webs even though the old value's last
            // read is BEFORE the source definition (double_reduction's four
            // accumulators lost their homes; got [] regression).
            let phi_live_in_window = liveness.folded_read_points.get(&dest.0).is_some_and(|pts| {
                windows
                    .iter()
                    .any(|&(lo, hi)| lo <= hi && pts.iter().any(|&p| lo <= p && p <= hi))
                    || mid_blocks.iter().any(|&m| {
                        let lo = liveness.block_starts[m];
                        let hi = liveness.block_ends[m];
                        pts.iter().any(|&p| lo <= p && p <= hi)
                    })
            });
            let source_used_before_copy = !same_block
                && block.instructions[..copy_idx]
                    .iter()
                    .any(|middle| uses_value(middle, src.0));
            if phi_used_in_window
                || phi_derived_used_in_window
                || phi_live_in_window
                || source_used_elsewhere
                || source_home_killed
                || source_used_before_copy
            {
                if debug {
                    eprintln!(
                        "[PHI_COALESCE] BLOCKED phi_dest=Value({}) src=Value({}) block={} source_block={} used_in_window={} derived_in_window={} live_in_window={} src_used_elsewhere={} src_home_killed={} src_before_copy={}",
                        dest.0,
                        src.0,
                        block_idx,
                        source_block,
                        phi_used_in_window,
                        phi_derived_used_in_window,
                        phi_live_in_window,
                        source_used_elsewhere,
                        source_home_killed,
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
            candidates.push(source_home_probe);
        }
    }

    // Chain application order (Part 2): a candidate whose backedge source
    // is another candidate's phi dest must come FIRST — the apply phase
    // hands the source the dest's CURRENT register, so the top of the
    // chain (farthest from the update) inherits before the links below
    // it, and the whole web `{acc, phi-dest, or-result}` collapses onto
    // one register (check_cpuflags: `{v46, v48}` before `{v48, v42}`).
    // `rank` = number of candidates strictly above this one in that
    // link; higher rank sorts first.
    let dest_index: FxHashMap<u32, Vec<usize>> =
        candidates
            .iter()
            .enumerate()
            .fold(FxHashMap::default(), |mut acc, (i, c)| {
                acc.entry(c.phi_dest).or_default().push(i);
                acc
            });
    let mut rank_memo: FxHashMap<usize, u32> = FxHashMap::default();
    for i in 0..candidates.len() {
        if rank_memo.contains_key(&i) {
            continue;
        }
        // Walk the chain above candidate i to its end or a memoized rank.
        let mut walk: Vec<usize> = vec![i];
        let mut cur = i;
        loop {
            if let Some(&base) = rank_memo.get(&cur) {
                let mut r = base;
                for idx in walk[..walk.len() - 1].iter().rev() {
                    r += 1;
                    rank_memo.insert(*idx, r);
                }
                break;
            }
            let next = dest_index
                .get(&candidates[cur].backedge_src)
                .and_then(|idxs| idxs.iter().max().copied());
            match next {
                Some(n) if n != cur && !walk.contains(&n) && walk.len() < 16 => {
                    walk.push(n);
                    cur = n;
                }
                _ => {
                    // Chain end (or depth cap / cycle): ranks run down
                    // from 0 at the bottom.
                    let mut r = 0u32;
                    for idx in walk.iter().rev() {
                        rank_memo.insert(*idx, r);
                        r += 1;
                    }
                    break;
                }
            }
        }
    }
    let mut ranked: Vec<(u32, PhiCoalesceCandidate)> = candidates
        .into_iter()
        .enumerate()
        .map(|(i, c)| (rank_memo[&i], c))
        .collect();
    // Hottest latch first: deeper loop, then later copy in the block
    // (latch sits after the body). Part 1 / claimed_dests first-wins.
    ranked.sort_by(|(ra, a), (rb, b)| {
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
        rb.cmp(ra)
            .then(db.cmp(&da))
            .then(a.copy_idx.cmp(&b.copy_idx).reverse())
            .then(a.phi_dest.cmp(&b.phi_dest))
    });
    let candidates: Vec<PhiCoalesceCandidate> = ranked.into_iter().map(|(_, c)| c).collect();
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
mod ra_config_tests {
    use super::{MI_MAX_LOOP_INSTS_DEFAULT, PhysReg, RaConfig};

    fn from(entries: &[(&str, &str)]) -> RaConfig {
        RaConfig::from_sources(
            |name| entries.iter().any(|(key, _)| *key == name),
            |name| {
                entries
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            },
        )
    }

    #[test]
    fn parser_covers_every_boolean_ra_switch_without_process_environment() {
        macro_rules! switch {
            ($field:ident, $name:literal) => {{
                assert!(!from(&[]).$field, "{} must default to disabled", $name);
                assert!(
                    from(&[($name, "enabled")]).$field,
                    "{} must be presence-enabled",
                    $name
                );
            }};
        }

        switch!(no_leaf_param_gpr, "CCC_NO_LEAF_PARAM_GPR");
        switch!(no_ir_divrem, "CCC_NO_IR_DIVREM");
        switch!(no_mulacc, "CCC_NO_MULACC");
        switch!(no_index_home, "CCC_NO_INDEX_HOME");
        switch!(no_folded_index_liveness, "CCC_NO_FOLDED_INDEX_LIVENESS");
        switch!(no_fold_addr_defer, "CCC_NO_FOLD_ADDR_DEFER");
        switch!(no_vecreg, "CCC_NO_VECREG");
        switch!(no_phi_coalesce, "CCC_NO_PHI_COALESCE");
        switch!(no_coalesce, "CCC_NO_COALESCE");
        switch!(no_hot_loop, "CCC_NO_HOT_LOOP");
        switch!(leaf_strict_call_free, "CCC_LEAF_STRICT_CALL_FREE");
        switch!(no_leaf_caller_home, "CCC_NO_LEAF_CALLER_HOME");
        // IPA-RA rides the shared exact-token pass list, not its own knob.
        assert!(!from(&[]).no_ipa_ra);
        assert!(from(&[("CCC_DISABLE_PASSES", "ipa_ra")]).no_ipa_ra);
        assert!(from(&[("CCC_DISABLE_PASSES", "gvn, ipa_ra")]).no_ipa_ra);
        assert!(from(&[("CCC_DISABLE_PASSES", "all")]).no_ipa_ra);
        assert!(!from(&[("CCC_DISABLE_PASSES", "ipa_ra_x,gvn")]).no_ipa_ra);
        switch!(no_span_valve, "CCC_NO_SPAN_VALVE");
        switch!(no_web_inloop_use, "CCC_NO_WEB_INLOOP_USE");
        switch!(no_recurrence_floor, "CCC_RA_NO_RECURRENCE_FLOOR");
        switch!(no_load_hazard_refine, "CCC_NO_LOAD_HAZARD_REFINE");
        switch!(no_eax_alloc, "CCC_NO_EAX_ALLOC");
        switch!(no_loop_pin, "CCC_NO_LOOP_PIN");
        switch!(no_hot_web_steal, "CCC_NO_HOT_WEB_STEAL");
        switch!(no_p1_waves, "CCC_NO_P1_WAVES");
        switch!(no_iterated_hazard, "CCC_NO_ITERATED_HAZARD");
        switch!(no_segment_fill, "CCC_NO_SEGMENT_FILL");
        switch!(no_reduction_vecreg, "CCC_NO_REDUCTION_VECREG");
        switch!(no_map_vecreg, "CCC_NO_MAP_VECREG");
        switch!(no_fp_copy_web, "CCC_NO_FP_COPY_WEB");
        switch!(fp_extract_homes, "CCC_FP_EXTRACT_HOMES");
        switch!(caller_save_spanning, "CCC_CALLER_SAVE_SPANNING");
        switch!(no_cold_call_split, "CCC_NO_COLD_CALL_SPLIT");
        switch!(no_phi_blocker_rehome, "CCC_NO_PHI_BLOCKER_REHOME");
        switch!(no_phase2c_reuse, "CCC_NO_PHASE2C_REUSE");
        switch!(no_callee_sticky, "CCC_NO_CALLEE_STICKY");
        switch!(no_save_set_compact, "CCC_NO_SAVE_SET_COMPACT");
        switch!(no_rdx_hazard, "CCC_NO_RDX_HAZARD");
        switch!(no_rdx_param_homes, "CCC_NO_RDX_PARAM_HOMES");
        switch!(no_segment_scan, "CCC_NO_SEGMENT_SCAN");
        switch!(debug_coalesce, "CCC_DEBUG_COALESCE");
        switch!(debug_coalesce_members, "CCC_DEBUG_COALESCE_MEMBERS");
        switch!(debug_phi_coalesce, "CCC_DEBUG_PHI_COALESCE");
        switch!(debug_ra_phases, "CCC_DEBUG_RA_PHASES");
        switch!(debug_ra_intervals, "CCC_DEBUG_RA_INTERVALS");
        switch!(debug_segment_fill, "CCC_DEBUG_SEGMENT_FILL");
        switch!(debug_ra, "CCC_DEBUG_RA");
        switch!(debug_ra_repair, "CCC_DEBUG_RA_REPAIR");
        switch!(debug_hazards, "CCC_DEBUG_HAZARDS");
        switch!(trace_alloc, "CCC_TRACE_ALLOC");
        switch!(trace_allocstats, "CCC_TRACE_ALLOCSTATS");
        switch!(verify_regalloc, "CCC_VERIFY_REGALLOC");
        switch!(legacy_debug_ra, "LCCC_DBG_RA");
        switch!(ra_explain_homes, "CCC_RA_EXPLAIN_HOMES");
        switch!(no_abi_reg_hints, "CCC_NO_ABI_REG_HINTS");
        switch!(disable_scalar_fp_xmm, "CCC_DISABLE_SCALAR_FP_XMM");
        switch!(enable_scalar_fp_xmm, "CCC_ENABLE_SCALAR_FP_XMM");
        switch!(no_xmm_regalloc, "CCC_NO_XMM_REGALLOC");
        switch!(no_promoted_fp_tail, "CCC_NO_PROMOTED_FP_TAIL");
        switch!(no_fp_callee_saved, "CCC_NO_FP_CALLEE_SAVED");
        switch!(no_regalloc, "CCC_NO_REGALLOC");
        switch!(dump_ir, "CCC_DUMP_IR");
        switch!(no_va_root_guard, "CCC_NO_VA_ROOT_GUARD");
        switch!(debug_vararg, "CCC_DEBUG_VARARG");
        switch!(no_x64_immed_nohome, "CCC_NO_X64_IMMED_NOHOME");
        switch!(no_i686_accum_nohome, "CCC_NO_I686_ACCUM_NOHOME");
        switch!(no_x64_push_arg_forms, "CCC_NO_X64_PUSH_ARG_FORMS");
        switch!(mi_all_classic, "CCC_MI_ALL_CLASSIC");
        switch!(mi_force_loops, "CCC_MI_FORCE_LOOPS");
        switch!(mi_debug, "CCC_MI_DEBUG");
        switch!(no_load_cast_fold, "CCC_NO_LOAD_CAST_FOLD");
        switch!(debug_load_cast_fold, "CCC_DEBUG_LOAD_CAST_FOLD");
        switch!(
            no_empty_local_frame_elision,
            "CCC_NO_EMPTY_LOCAL_FRAME_ELISION"
        );
        switch!(debug_param_store, "CCC_DEBUG_PARAM_STORE");
        switch!(debug_paramref, "CCC_DEBUG_PARAMREF");
        switch!(no_machinst, "CCC_NO_MACHINST");
    }

    #[test]
    fn parser_preserves_numeric_text_and_polarity_contracts() {
        let defaults = from(&[]);
        assert_eq!(defaults.loop_pin, 2);
        assert_eq!(defaults.hot_web_steal, 3);
        assert_eq!(defaults.evict_mode, 3);
        assert_eq!(defaults.evict_short_k, 16);
        assert_eq!(defaults.pgo_weight_max, 1);
        assert_eq!(defaults.loop_span_reserve, 0);
        assert_eq!(
            defaults.x64_nohome_classes,
            "ret,store,copy,cast,unary,binop"
        );
        assert_eq!(defaults.mi_max_loop_insts, MI_MAX_LOOP_INSTS_DEFAULT);
        assert_eq!(defaults.legacy_debug_ra_func, "");
        assert_eq!(defaults.mi_fn_disable, "");
        assert_eq!(defaults.mi_fn_force, "");
        assert_eq!(defaults.mi_disable_kinds, "");
        assert_eq!(defaults.trace_allocstats_filter, None);
        assert_eq!(defaults.dump_ir_func, None);
        assert_eq!(defaults.no_regalloc_func, None);
        assert_eq!(defaults.ra_explain, None);
        assert_eq!(defaults.ra_drop, None);
        assert_eq!(defaults.ra_drop_func, None);
        assert_eq!(defaults.phi_coalesce_skip, None);
        assert_eq!(defaults.phi_coalesce_func, None);
        assert_eq!(defaults.segment_fill_limit, None);
        assert_eq!(defaults.segment_fill_func, None);

        let configured = from(&[
            ("CCC_LOOP_PIN", "7"),
            ("CCC_HOT_WEB_STEAL", "9"),
            ("CCC_RA_LOOP_SPAN_RESERVE", "2"),
            ("CCC_EVICT_MODE", "6"),
            ("CCC_PGO_WEIGHT_MAX", "99"),
            ("CCC_X64_NOHOME_CLASSES", "ret,cast"),
            ("CCC_MI_MAX_LOOP_INSTS", "41"),
            ("CCC_DUMP_IR_FUNC", "dump_only_this"),
            ("CCC_TRACE_ALLOCSTATS", "ra_fn"),
            ("LCCC_DBG_RA_FUNC", "legacy_fn"),
            ("CCC_RA_EXPLAIN", "explain_fn"),
            ("CCC_RA_DROP", "3,5"),
            ("CCC_RA_DROP_FUNC", "drop_fn"),
            ("CCC_PHI_COALESCE_SKIP", "7,11"),
            ("CCC_PHI_COALESCE_FUNC", "phi_fn"),
            ("CCC_NO_REGALLOC_FUNC", "slow_fn,other_fn"),
            ("CCC_MI_FN_DISABLE", "cold"),
            ("CCC_MI_FN_FORCE", "hot"),
            ("CCC_MI_DISABLE_KINDS", "call,load"),
            ("CCC_SEGMENT_FILL_LIMIT", "4"),
            ("CCC_SEGMENT_FILL_FUNC", "fill_fn"),
        ]);
        assert_eq!(configured.loop_pin, 7);
        assert_eq!(configured.hot_web_steal, 9);
        assert_eq!(configured.evict_mode, 6);
        assert_eq!(configured.loop_span_reserve, 2);
        assert_eq!(configured.pgo_weight_max, 16);
        assert_eq!(configured.x64_nohome_classes, "ret,cast");
        assert_eq!(configured.mi_max_loop_insts, 41);
        assert_eq!(configured.dump_ir_func.as_deref(), Some("dump_only_this"));
        assert_eq!(configured.trace_allocstats_filter.as_deref(), Some("ra_fn"));
        assert_eq!(configured.legacy_debug_ra_func, "legacy_fn");
        assert_eq!(configured.ra_explain.as_deref(), Some("explain_fn"));
        assert_eq!(configured.ra_drop.as_deref(), Some("3,5"));
        assert_eq!(configured.ra_drop_func.as_deref(), Some("drop_fn"));
        assert_eq!(configured.phi_coalesce_skip.as_deref(), Some("7,11"));
        assert_eq!(configured.phi_coalesce_func.as_deref(), Some("phi_fn"));
        assert_eq!(
            configured.no_regalloc_func.as_deref(),
            Some("slow_fn,other_fn")
        );
        assert_eq!(configured.mi_fn_disable, "cold");
        assert_eq!(configured.mi_fn_force, "hot");
        assert_eq!(configured.mi_disable_kinds, "call,load");
        assert_eq!(configured.segment_fill_limit, Some(4));
        assert_eq!(configured.segment_fill_func.as_deref(), Some("fill_fn"));

        let malformed = from(&[
            ("CCC_LOOP_PIN", "not-a-number"),
            ("CCC_HOT_WEB_STEAL", "-1"),
            ("CCC_EVICT_MODE", "bad"),
            ("CCC_RA_LOOP_SPAN_RESERVE", "not-a-number"),
            ("CCC_PGO_WEIGHT_MAX", "0"),
            ("CCC_MI_MAX_LOOP_INSTS", "bad"),
            ("CCC_SEGMENT_FILL_LIMIT", "-3"),
        ]);
        assert_eq!(malformed.loop_pin, 2);
        assert_eq!(malformed.hot_web_steal, 3);
        assert_eq!(malformed.evict_mode, 3);
        assert_eq!(malformed.loop_span_reserve, 0);
        assert_eq!(malformed.pgo_weight_max, 1);
        assert_eq!(malformed.mi_max_loop_insts, MI_MAX_LOOP_INSTS_DEFAULT);
        assert_eq!(malformed.segment_fill_limit, None);

        let fp_override = from(&[
            ("CCC_DISABLE_SCALAR_FP_XMM", "1"),
            ("CCC_ENABLE_SCALAR_FP_XMM", "1"),
        ]);
        assert!(fp_override.disable_scalar_fp_xmm);
        assert!(fp_override.enable_scalar_fp_xmm);
    }

    #[test]
    fn explicit_config_controls_linear_scan_without_process_environment() {
        use crate::backend::live_range::{LinearScanAllocator, LiveRange};
        use crate::backend::liveness::LiveInterval;
        use std::sync::Arc;

        let mut range = LiveRange::from_interval(
            LiveInterval {
                value_id: 1,
                start: 0,
                end: 10,
            },
            0,
        );
        range.set_segments(vec![(0, 2), (8, 10)]);
        let defaults = Arc::new(from(&[]));
        let disabled = Arc::new(from(&[("CCC_NO_SEGMENT_SCAN", "1")]));

        let enabled_scan =
            LinearScanAllocator::new_with_config(vec![range.clone()], vec![PhysReg(1)], &defaults);
        let disabled_scan =
            LinearScanAllocator::new_with_config(vec![range], vec![PhysReg(1)], &disabled);
        assert!(enabled_scan.segment_mode);
        assert!(!disabled_scan.segment_mode);
    }

    #[test]
    fn parser_instances_are_isolated() {
        let enabled = from(&[
            ("CCC_NO_SEGMENT_SCAN", "1"),
            ("CCC_EVICT_MODE", "5"),
            ("CCC_RA_EXPLAIN", "only_here"),
        ]);
        let defaults = from(&[]);

        assert!(enabled.no_segment_scan);
        assert_eq!(enabled.evict_mode, 5);
        assert_eq!(enabled.ra_explain.as_deref(), Some("only_here"));
        assert!(!defaults.no_segment_scan);
        assert_eq!(defaults.evict_mode, 3);
        assert_eq!(defaults.ra_explain, None);
    }
}

#[cfg(test)]
mod phi_coalesce_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrCmpOp, Value};

    #[test]
    fn segment_interference_preserves_holes_and_half_open_handoffs() {
        assert!(!segment_sets_overlap(&[(1, 3), (8, 10)], &[(3, 8)]));
        let guarded = guard_internal_hole_boundaries(&[(1, 3), (8, 10)]);
        assert_eq!(guarded, vec![(1, 4), (7, 10)]);
        assert!(segment_sets_overlap(&guarded, &[(3, 5)]));
        assert!(segment_sets_overlap(&guarded, &[(6, 8)]));
        assert!(!segment_sets_overlap(&[(1, 3)], &[(3, 8)]));
        assert!(!segment_sets_overlap(&[(1, 5)], &[(5, 9)]));
        assert!(segment_sets_overlap(&[(1, 5), (9, 12)], &[(4, 7)]));
        assert!(segment_sets_overlap(&[(1, 2), (6, 9)], &[(3, 7)]));

        let mut occupied = vec![(1, 3), (8, 10)];
        insert_segment_union(&mut occupied, &[(3, 5), (6, 8), (12, 14)]);
        assert_eq!(occupied, vec![(1, 5), (6, 10), (12, 14)]);
    }

    #[test]
    fn segment_fill_admission_is_order_independent_at_internal_holes() {
        // A is live on [(1,3),(8,10)] with internal hole (3,8); B is exactly
        // [(3,8)]. The guard law forbids B from touching both of A's hole
        // boundaries, so whichever of the two is placed first, the other must
        // be refused the same register. (The pre-fix Phase 2f inserted the
        // accepted candidate's RAW segments into occupancy: A-then-B admitted
        // both, B-then-A refused A — an order-dependent interference hole.)
        let pool = [PhysReg(1)];
        let a = [(1, 3), (8, 10)];
        let b = [(3, 8)];

        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        assert_eq!(
            try_fill_segment_candidate(&a, &mut occupied, &pool),
            Some(PhysReg(1))
        );
        assert_eq!(try_fill_segment_candidate(&b, &mut occupied, &pool), None);

        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        assert_eq!(
            try_fill_segment_candidate(&b, &mut occupied, &pool),
            Some(PhysReg(1))
        );
        assert_eq!(try_fill_segment_candidate(&a, &mut occupied, &pool), None);

        // The guard is exactly one point on each internal boundary, not a
        // blanket ban on hole reuse: [(4,7)] sits strictly inside A's hole
        // with the required clearance and must still be admitted.
        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        assert_eq!(
            try_fill_segment_candidate(&a, &mut occupied, &pool),
            Some(PhysReg(1))
        );
        assert_eq!(
            try_fill_segment_candidate(&[(4, 7)], &mut occupied, &pool),
            Some(PhysReg(1))
        );
    }

    #[test]
    fn attach_scan_segments_keeps_envelope_for_single_segment_values() {
        // The hole-aware scan exists to model real gaps. A value whose
        // coverage is ONE contiguous segment strictly inside its fat
        // envelope keeps envelope semantics: shrinking to touch-based
        // coverage would drop the envelope fringe other allocator phases
        // deliberately widened (live-through pulls, hidden reads). A value
        // with an actual hole gets its segments attached.
        use crate::backend::live_range::LiveRange;
        let iv = |value_id, start, end| LiveInterval {
            value_id,
            start,
            end,
        };
        let mut func = IrFunction::new("attach_fixture".to_string(), IrType::Void, vec![], false);
        func.blocks = vec![block(0, Vec::new(), Terminator::Return(None))];
        let mut liveness = compute_live_intervals(&func);
        liveness.intervals = vec![iv(1, 0, 20), iv(2, 0, 20)];
        liveness.segments = vec![iv(1, 4, 16), iv(2, 2, 6), iv(2, 12, 18)];

        let mut ranges = vec![
            LiveRange::from_interval(iv(1, 0, 20), 0),
            LiveRange::from_interval(iv(2, 0, 20), 0),
        ];
        attach_scan_segments(&mut ranges, &liveness, &FxHashMap::default());

        assert!(
            ranges[0].segments.is_empty(),
            "single-segment value must keep fat envelope semantics"
        );
        assert_eq!(ranges[1].segments, vec![(2, 6), (12, 18)]);
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
        let loop_depth = liveness.block_loop_depth.first().copied().unwrap_or(0);
        assert!(
            loop_depth > 0,
            "self-backedge must have positive loop depth, got {loop_depth}"
        );
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

    // Part 2 (FOLLOWUP-2026-09-22B §4): the cpucheck `err |= 1 << i`
    // split-latch shape.  v1 = acc, v2 = i;
    //   block 2 (body): cond -> 3 / 4
    //   block 3 (orpath): v5 = 1 << v2; v6 = v1 | v5; v7 = v6
    //   block 4 (skip):   v7 = v1                (passthrough)
    //   block 5 (latch):  v8 = v2 + 1; v1 = v7; v2 = v8
    // `mutate` lets the rejection tests reshape blocks in place.
    fn part2_shape(mut mutate: impl FnMut(&mut [BasicBlock])) -> Vec<PhiCoalesceCandidate> {
        let mut func = IrFunction::new("part2_cpucheck".to_string(), IrType::U32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(8)),
                    ty: IrType::U32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(2),
                    false_label: BlockId(6),
                },
            ),
            block(
                2,
                vec![Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ),
            block(
                3,
                vec![
                    Instruction::BinOp {
                        dest: Value(5),
                        op: IrBinOp::Shl,
                        lhs: Operand::Const(IrConst::I64(1)),
                        rhs: Operand::Value(Value(2)),
                        ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: Value(6),
                        op: IrBinOp::Or,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Value(Value(5)),
                        ty: IrType::U32,
                    },
                    Instruction::Copy {
                        dest: Value(7),
                        src: Operand::Value(Value(6)),
                    },
                ],
                Terminator::Branch(BlockId(5)),
            ),
            block(
                4,
                vec![Instruction::Copy {
                    dest: Value(7),
                    src: Operand::Value(Value(1)),
                }],
                Terminator::Branch(BlockId(5)),
            ),
            block(
                5,
                vec![
                    Instruction::BinOp {
                        dest: Value(8),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Const(IrConst::I64(1)),
                        ty: IrType::U32,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(7)),
                    },
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Value(Value(8)),
                    },
                    Instruction::Cmp {
                        dest: Value(9),
                        op: IrCmpOp::Ult,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Const(IrConst::I64(8)),
                        ty: IrType::U32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(9)),
                    true_label: BlockId(2),
                    false_label: BlockId(6),
                },
            ),
            block(
                6,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        mutate(&mut func.blocks);
        func.next_value_id = 10;
        let liveness = compute_live_intervals(&func);
        detect_phi_coalesce_groups(&func, &liveness)
    }

    #[test]
    fn accepts_part2_split_latch_chain() {
        // The kernel `check_cpuflags` web: `{acc v1, phi-dest v7,
        // or-result v6}` must chain — the Part-2 latch candidate
        // `{v1, v7}` on `u`'s (v6's) site, plus the pre-existing
        // single-def `{v7, v6}`, and the chain candidate must sort FIRST
        // so the apply phase cascades the register down the web.
        let cands = part2_shape(|_| {});
        let part2 = cands
            .iter()
            .position(|c| c.phi_dest == 1 && c.backedge_src == 7)
            .expect("Part-2 latch candidate {{v1, v7}} missing: {cands:?}");
        let c = &cands[part2];
        assert_eq!(c.block_idx, 5, "copy block must be the latch: {cands:?}");
        assert_eq!(
            c.source_block_idx, 3,
            "u's block must be the or-path: {cands:?}"
        );
        assert_eq!(
            c.source_def_idx, 2,
            "candidate must record v7's def (copy v7 = v6), not u's own def: {cands:?}"
        );
        assert_eq!(
            c.copy_idx, 1,
            "copy v1 = v7 is latch instruction 1: {cands:?}"
        );
        let link = cands
            .iter()
            .position(|c| c.phi_dest == 7 && c.backedge_src == 6)
            .expect("chained single-def candidate {{v7, v6}} missing: {cands:?}");
        assert!(
            part2 < link,
            "chain top must sort first so the register cascades: part2@{part2} link@{link}"
        );
        assert!(
            cands.iter().any(|c| c.phi_dest == 2 && c.backedge_src == 8),
            "counter web {{v2, v8}} must still coalesce: {cands:?}"
        );
    }

    #[test]
    fn rejects_part2_when_other_pred_lacks_passthrough() {
        // The skip path stops defining v7: the latch copy v1 = v7 would
        // read an undefined/other value on that path.
        let cands = part2_shape(|blocks| {
            blocks[4].instructions.clear();
        });
        assert!(
            !cands.iter().any(|c| c.phi_dest == 1 && c.backedge_src == 7),
            "latch copy must not coalesce without the passthrough on every other pred: {cands:?}"
        );
    }

    #[test]
    fn rejects_part2_when_m_has_non_copy_def() {
        // v7 is also computed (not copied) on the skip path: the
        // all-defs-are-Copies clause must reject.
        let cands = part2_shape(|blocks| {
            blocks[4].instructions = vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(1)),
                ty: IrType::U32,
            }];
        });
        assert!(
            !cands.iter().any(|c| c.phi_dest == 1 && c.backedge_src == 7),
            "non-Copy def of the multi-def source must block Part-2: {cands:?}"
        );
    }

    #[test]
    fn rejects_part2_with_two_non_passthrough_defs() {
        // Both preds update v7 from different values (v6 and v5): more
        // than one non-passthrough def.
        let cands = part2_shape(|blocks| {
            blocks[4].instructions = vec![Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(5)),
            }];
        });
        assert!(
            !cands.iter().any(|c| c.phi_dest == 1 && c.backedge_src == 7),
            "two non-passthrough defs must block Part-2: {cands:?}"
        );
    }

    #[test]
    fn rejects_part2_when_u_is_multi_def() {
        // v6 (u) is defined in both the body and the or-path: the window
        // proof has no unique `u` site.
        let cands = part2_shape(|blocks| {
            blocks[2].instructions.push(Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(5)),
                ty: IrType::U32,
            });
        });
        assert!(
            !cands.iter().any(|c| c.phi_dest == 1 && c.backedge_src == 7),
            "multi-def u must block Part-2: {cands:?}"
        );
    }

    #[test]
    fn rejects_part2_on_phi_use_between_u_and_copy() {
        // The old acc is still read in the latch before the update copy:
        // the cross-block window proof at `u`'s site must reject.
        let cands = part2_shape(|blocks| {
            let mut latch = blocks[5].instructions.clone();
            latch.insert(
                0,
                Instruction::Copy {
                    dest: Value(10),
                    src: Operand::Value(Value(1)),
                },
            );
            blocks[5].instructions = latch;
        });
        assert!(
            !cands.iter().any(|c| c.phi_dest == 1 && c.backedge_src == 7),
            "old-acc read inside the update window must block Part-2: {cands:?}"
        );
    }

    #[test]
    fn coalesces_integer_split_latch_counter() {
        // `do { ... } while (--n)`: the update `n2 = n1 - 1` ends the body,
        // the phi-elim copy sits alone in the one-predecessor split latch,
        // and nothing reads the OLD n after the destructive update (the exit
        // returns a constant).  Pre-Fable this shape was restricted to FP
        // sources, leaving 2 copies per iteration in every counted loop.
        let mut func = IrFunction::new(
            "split_latch_counter".to_string(),
            IrType::I32,
            vec![],
            false,
        );
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(10)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                Vec::new(),
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        func.next_value_id = 4;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "integer split-latch counter must coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_split_latch_when_old_phi_read_on_exit() {
        // `for (i = 0; i < n; i++) ...; return i;` — the exit path reads the
        // phi AFTER the destructive update in the body; sharing the homes
        // would hand the exit the POST-increment value.  The exit block
        // reads the phi, so `phi_escapes` must block the candidate.
        let mut func = IrFunction::new(
            "split_latch_return_phi".to_string(),
            IrType::I32,
            vec![],
            false,
        );
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
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
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
            "old phi live into the exit block must block coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_split_latch_when_gp_derived_value_escapes() {
        // `p = &a[i]` (derived from the phi) formed before the update and
        // consumed in a successor: a folded GEP would re-read the phi's
        // register and observe the NEW i. Only address-forming instructions
        // (Cast/GEP) keep the register dependency; a BinOp would have been
        // materialized at its own point and is therefore a legal coalesce
        // (see `accepts_materialized_binop_escaping_source_block`).
        let mut func = IrFunction::new(
            "split_latch_derived_escape".to_string(),
            IrType::I32,
            vec![],
            false,
        );
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(10)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::GetElementPtr {
                        dest: Value(3),
                        base: Value(1),
                        offset: Operand::Const(IrConst::I32(2)),
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![Instruction::Load {
                    dest: Value(4),
                    ptr: Value(3),
                    ty: IrType::I32,
                    seg_override: crate::common::types::AddressSpace::Default,
                    volatile: false,
                }],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        func.next_value_id = 5;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            !candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "derived GEP live out of the source block must block coalesce: {candidates:?}"
        );
    }

    #[test]
    fn accepts_materialized_binop_escaping_source_block() {
        // The adler-style shape: a value computed from the phi BEFORE the
        // update by a BinOp (materialized into its own register at its own
        // point — no deferred address re-formation) and consumed in a
        // successor. Coalescing is legal: the consumer reads the
        // materialized register, not the phi's home.  (`v3 = n1 * 2`
        // followed by `return v3` after the exit branch.)
        let mut func = IrFunction::new(
            "split_latch_binop_escape".to_string(),
            IrType::I32,
            vec![],
            false,
        );
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(10)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(2)),
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        func.next_value_id = 5;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "materialized (non-address) derived values must not veto coalescing: {candidates:?}"
        );
    }

    #[test]
    fn rejects_const_binop_chain_folding_into_dereferenced_gep() {
        // Mirror of accepts_materialized_binop_escaping_source_block: the SAME
        // constant-operand binop shape, but here the derived value feeds a
        // GetElementPtr that a Load/Store dereferences on the escape path.
        // The backend may absorb the whole chain into the access's SIB
        // operand and re-read the phi's register there, so the destructive
        // coalesce MUST stay vetoed (the addr_fed gate narrows the binop
        // peel to address-fed chains, it does not remove it).
        let mut func = IrFunction::new(
            "folded_binop_index_escape".to_string(),
            IrType::I32,
            vec![],
            false,
        );
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(10)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(2)),
                        ty: IrType::I32,
                    },
                    Instruction::GlobalAddr {
                        dest: Value(4),
                        name: "xs".to_string(),
                    },
                    Instruction::GetElementPtr {
                        dest: Value(5),
                        base: Value(4),
                        offset: Operand::Value(Value(3)),
                        ty: IrType::Ptr,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I8(7)),
                    ptr: Value(5),
                    ty: IrType::I8,
                    seg_override: crate::common::types::AddressSpace::Default,
                }],
                Terminator::Return(None),
            ),
        ];
        func.next_value_id = 6;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            !candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "address-fed const-binop chain must still veto coalescing: {candidates:?}"
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
        let loop_depth = liveness.block_loop_depth.get(1).copied().unwrap_or(0);
        assert!(
            loop_depth > 0,
            "inner latch must have positive loop depth, got {loop_depth}"
        );
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        let dest1: Vec<_> = candidates.iter().filter(|c| c.phi_dest == 1).collect();
        assert!(
            dest1.len() >= 2,
            "two latches must both be candidates: {candidates:?}"
        );
        assert_eq!(
            dest1[0].backedge_src, 3,
            "later latch must sort first: {candidates:?}"
        );
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
        let (preds, _) = analysis::build_cfg(&func, &analysis::build_label_map(&func));
        assert!(
            phi_window_clobbers_caller_saved(&func, &cand, &preds),
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

    /// PHI-COALESCE-HOME-CONFLICT fixture (zlib_ng_adler32's checksum loop):
    /// `v1` is the loop phi, `v2 = v1 + 1` its backedge source, live out of
    /// the loop into B3, where the post-loop temporary `v5` was handed the
    /// phi's home. `exit_call` puts a call between `v5`'s def and its last
    /// use (after `v2` died) so the blocker meets a call point.
    fn rehome_fixture(exit_call: bool) -> (IrFunction, PhiCoalesceCandidate) {
        let mut func = IrFunction::new("rehome".to_string(), IrType::I32, vec![], false);
        let mut exit = vec![
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(36)),
                ty: IrType::I32,
            },
        ];
        // `v7` keeps the call strictly after `v2`'s last use, so only the
        // blocker -- not the backedge source -- meets the call point.
        exit.push(Instruction::BinOp {
            dest: Value(7),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(4)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        if exit_call {
            exit.push(empty_call(None));
        }
        exit.push(Instruction::BinOp {
            dest: Value(8),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(5)),
            rhs: Operand::Value(Value(7)),
            ty: IrType::I32,
        });
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(6),
                        src: Operand::Const(IrConst::I32(0)),
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(6)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, exit, Terminator::Return(Some(Operand::Value(Value(8))))),
        ];
        func.next_value_id = 9;
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 2,
            source_block_idx: 1,
            source_def_idx: 0,
            copy_idx: 0,
        };
        (func, cand)
    }

    /// Runs the production apply path (blocker re-homing enabled) on x86-64.
    fn apply_with_rehome(
        func: &IrFunction,
        cand: PhiCoalesceCandidate,
        assignments: &mut FxHashMap<u32, PhysReg>,
        callee_saved: &[PhysReg],
        member_of: &FxHashMap<u32, u32>,
    ) -> usize {
        use crate::common::types::{set_target_elf_machine, target_elf_machine};
        let liveness = compute_live_intervals(func);
        let iv_map = interval_map(&liveness);
        let before = assignments.clone();
        let saved = target_elf_machine();
        set_target_elf_machine(crate::backend::elf::EM_X86_64);
        // r11, r10, r8, r9, rdi, rsi
        let caller_saved = [10, 11, 12, 13, 14, 15].map(PhysReg);
        let applied = apply_phi_coalesce_assignments_with_config(
            func,
            &liveness,
            &iv_map,
            &[cand],
            assignments,
            callee_saved,
            Some((member_of, &caller_saved)),
            &RaConfig::default(),
        );
        set_target_elf_machine(saved);
        // Whatever moved, no two distinct homes may overlap (inclusive),
        // except the applied pair itself and co-tenants the input already
        // had (a two-address chain touching at its death point keeps that
        // relation wherever it moves together).
        for (&a, &ra) in assignments.iter() {
            for (&b, &rb) in assignments.iter() {
                let pair = applied.iter().any(|p| {
                    [p.phi_dest, p.backedge_src] == [a, b] || [p.backedge_src, p.phi_dest] == [a, b]
                });
                let co_tenants = before.get(&a).is_some_and(|r| before.get(&b) == Some(r));
                if a < b && ra == rb && !pair && !co_tenants {
                    let (ia, ib) = (iv_map[&a], iv_map[&b]);
                    assert!(
                        ia.1 < ib.0 || ib.1 < ia.0,
                        "v{a} {ia:?} and v{b} {ib:?} share r{}: {assignments:?}",
                        ra.0
                    );
                }
            }
        }
        applied.len()
    }

    #[test]
    fn phi_blocker_rehome_moves_a_post_loop_temp_out_of_the_phi_home() {
        let (func, cand) = rehome_fixture(false);
        let initial: FxHashMap<u32, PhysReg> = [
            (6, PhysReg(1)),  // rbx: the preheader constant
            (1, PhysReg(14)), // phi: %rdi
            (2, PhysReg(10)), // backedge source: %r11 -> latch copy
            (5, PhysReg(14)), // post-loop temp in the phi's home: blocker
            (4, PhysReg(12)),
            (7, PhysReg(15)),
            (8, PhysReg(13)),
        ]
        .into_iter()
        .collect();

        // The opt-out wrapper keeps the historical veto (and a real blocker
        // still blocks: the conflict test is not inverted).
        let mut legacy = initial.clone();
        let liveness = compute_live_intervals(&func);
        apply_phi_coalesce_assignments(
            &func,
            &liveness,
            &interval_map(&liveness),
            &[cand],
            &mut legacy,
            &[PhysReg(1)],
        );
        assert_eq!(legacy, initial, "no re-homing without the context");

        let mut moved = initial.clone();
        let n = apply_with_rehome(
            &func,
            cand,
            &mut moved,
            &[PhysReg(1)],
            &FxHashMap::default(),
        );
        assert_eq!(n, 1, "the pair coalesces once the blocker moves: {moved:?}");
        assert_eq!(moved[&2], PhysReg(14), "source inherits the phi home");
        // rbx holds v6 [0, 1] only: free over v5, and already saved.
        assert_eq!(moved[&5], PhysReg(1), "callee-saved tier first: {moved:?}");

        // Without a used callee-saved register the caller-saved tier serves
        // (r11 still holds v2 at decision time and overlaps; r10 is free).
        let mut caller = initial.clone();
        caller.remove(&6);
        let n = apply_with_rehome(
            &func,
            cand,
            &mut caller,
            &[PhysReg(1)],
            &FxHashMap::default(),
        );
        assert_eq!(n, 1, "{caller:?}");
        assert_eq!(
            caller[&5],
            PhysReg(11),
            "unused rbx is never chosen: {caller:?}"
        );
    }

    /// adler32's post-loop chain: `v5 = v2 + 7; v9 = v5 + 1` touch at v5's
    /// death point and BOTH sit in the phi home while v2 is live. They are
    /// already proven compatible there, so they must be able to move into
    /// one target together; judging them against each other (inclusive)
    /// split the chain and the pair stayed blocked.
    fn rehome_chain_fixture() -> (IrFunction, PhiCoalesceCandidate) {
        let (mut func, cand) = rehome_fixture(false);
        let bin = |dest: u32, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty: IrType::I32,
        };
        func.blocks[3].instructions = vec![
            bin(5, Operand::Value(Value(2)), Operand::Const(IrConst::I32(7))),
            bin(9, Operand::Value(Value(5)), Operand::Const(IrConst::I32(1))),
            Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(36)),
                ty: IrType::I32,
            },
            bin(8, Operand::Value(Value(9)), Operand::Value(Value(4))),
        ];
        func.next_value_id = 10;
        (func, cand)
    }

    #[test]
    fn phi_blocker_rehome_moves_a_touching_chain_together() {
        let (func, cand) = rehome_chain_fixture();
        let initial: FxHashMap<u32, PhysReg> = [
            (6, PhysReg(1)),  // rbx: preheader constant, already saved
            (1, PhysReg(14)), // phi: %rdi
            (2, PhysReg(10)), // backedge source
            (5, PhysReg(14)), // chain head  } both block the pair
            (9, PhysReg(14)), // chain tail  }
            (4, PhysReg(12)),
            (8, PhysReg(13)),
        ]
        .into_iter()
        .collect();
        let mut moved = initial.clone();
        let n = apply_with_rehome(
            &func,
            cand,
            &mut moved,
            &[PhysReg(1)],
            &FxHashMap::default(),
        );
        assert_eq!(n, 1, "{moved:?}");
        assert_eq!(moved[&2], PhysReg(14), "source inherits the phi home");
        assert_eq!(moved[&5], PhysReg(1), "{moved:?}");
        assert_eq!(moved[&9], PhysReg(1), "the chain was split: {moved:?}");
    }

    #[test]
    fn phi_blocker_rehome_fails_closed() {
        let (func, cand) = rehome_fixture(false);
        let initial: FxHashMap<u32, PhysReg> = [
            (1, PhysReg(14)),
            (2, PhysReg(10)),
            (5, PhysReg(14)),
            (4, PhysReg(12)),
            (7, PhysReg(15)),
            (8, PhysReg(13)),
        ]
        .into_iter()
        .collect();
        // A copy-coalesce member/leader is re-propagated from its leader
        // later: it never moves, so the pair stays blocked.
        let mut grouped = initial.clone();
        let member_of: FxHashMap<u32, u32> = [(5, 8)].into_iter().collect();
        assert_eq!(
            apply_with_rehome(&func, cand, &mut grouped, &[], &member_of),
            0
        );
        assert_eq!(grouped, initial);

        // A blocker meeting a call point never takes a caller-saved home
        // (argument staging / call clobbers live there), and with no used
        // callee-saved register there is no other tier.
        let (func, cand) = rehome_fixture(true);
        let mut at_call = initial.clone();
        assert_eq!(
            apply_with_rehome(
                &func,
                cand,
                &mut at_call,
                &[PhysReg(1)],
                &FxHashMap::default()
            ),
            0
        );
        assert_eq!(at_call, initial);
    }

    /// loop_rotate_seq_loops shape: the rotated loop's backedge source `s`
    /// is LIVE ACROSS the exit block's printf call, but the (def, copy)
    /// window itself contains no call.  Re-homing `s` into the phi's
    /// caller-saved register makes the post-call `s != 36` Cmp read the
    /// callee's leftovers.  The apply phase must consult the source's full
    /// interval, not just the window.
    #[test]
    fn apply_refuses_caller_saved_home_across_call_outside_window() {
        let mut func = IrFunction::new("rotate_seq_shape".to_string(), IrType::I32, vec![], false);
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
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![
                    empty_call(None),
                    Instruction::Cmp {
                        dest: Value(4),
                        op: IrCmpOp::Ne,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Const(IrConst::I32(36)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        func.next_value_id = 5;

        let liveness = compute_live_intervals(&func);
        let iv_map = interval_map(&liveness);
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 2,
            source_block_idx: 1,
            source_def_idx: 0,
            copy_idx: 0,
        };
        let (preds, _) = analysis::build_cfg(&func, &analysis::build_label_map(&func));
        assert!(
            !phi_window_clobbers_caller_saved(&func, &cand, &preds),
            "precondition: the call sits in the exit block, outside the (def, copy) window"
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
            "post-call use of the source must veto the caller-saved home: {assignments:?}"
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

    /// Same shape minus the call: the source dies in the exit block's
    /// arithmetic, so the caller-saved home is legal.  The interval guard
    /// must not over-block when no call point falls inside the source range.
    #[test]
    fn apply_accepts_caller_saved_home_when_no_call_in_source_range() {
        let mut func = IrFunction::new("rotate_seq_nocall".to_string(), IrType::I32, vec![], false);
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
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(3)),
                    ty: IrType::I32,
                }],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        func.next_value_id = 5;

        let liveness = compute_live_intervals(&func);
        let iv_map = interval_map(&liveness);
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 2,
            source_block_idx: 1,
            source_def_idx: 0,
            copy_idx: 0,
        };

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
            assignments.get(&2).copied(),
            Some(PhysReg(14)),
            "call-free source range must still coalesce: {assignments:?}"
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
            ipa_regs: Vec::new(),
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
            fold_base_regs: Vec::new(),
            fold_readmit: FxHashSet::default(),
            reg_hints: FxHashMap::default(),
            ra_config: Arc::new(RaConfig::default()),
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

    /// Split-latch loop helper:
    ///   0: v1 = 0            (preheader init)
    ///   1: v2 = v1 + 1; condbr v2 -> 2 (latch) / 3 (exit)
    ///   2: v1 = v2; branch 1 (single-predecessor split latch)
    ///   3: exit
    fn split_latch_func(exit_ret: Terminator) -> IrFunction {
        let mut func = IrFunction::new("split_latch".to_string(), IrType::I32, vec![], false);
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
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, Vec::new(), exit_ret),
        ];
        func.next_value_id = 3;
        func
    }

    fn coalesced_pair(candidates: &[PhiCoalesceCandidate]) -> Option<(usize, usize)> {
        candidates
            .iter()
            .find(|c| c.phi_dest == 1 && c.backedge_src == 2)
            .map(|c| (c.block_idx, c.source_block_idx))
    }

    #[test]
    fn accepts_split_latch_int_backedge_coalesce() {
        // The rotated counted-loop shape with an INTEGER carried value:
        // the latch is a separate single-predecessor block and the exit
        // consumes nothing.  This used to be refused for non-FP sources,
        // leaving 2 movs + a jmp per iteration (zlib-ng adler32 shape).
        let func = split_latch_func(Terminator::Return(Some(Operand::Const(IrConst::I32(0)))));
        let liveness = compute_live_intervals(&func);
        assert!(
            liveness.block_loop_depth.get(1).copied().unwrap_or(0) > 0,
            "loop-depth oracle must mark the split-latch loop (test precondition)"
        );
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert_eq!(
            coalesced_pair(&candidates),
            Some((2, 1)),
            "integer split-latch backedge must coalesce: {candidates:?}"
        );
    }

    #[test]
    fn accepts_exit_block_src_use_when_path_clean() {
        // `return b` after the last iteration: the exit successor reads the
        // freshly computed backedge source.  The path def->use passes no
        // other phi definition, so the shared home holds exactly the value
        // the exit consumes.
        let func = split_latch_func(Terminator::Return(Some(Operand::Value(Value(2)))));
        let liveness = compute_live_intervals(&func);
        assert!(
            liveness.block_loop_depth.get(1).copied().unwrap_or(0) > 0,
            "loop-depth oracle must mark the split-latch loop (test precondition)"
        );
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert_eq!(
            coalesced_pair(&candidates),
            Some((2, 1)),
            "clean exit-edge use of the source must coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_exit_block_old_phi_use() {
        // `return i` after `for (...; ++i)`: the exit successor reads the
        // OLD phi value, which the destructive update would clobber.
        let func = split_latch_func(Terminator::Return(Some(Operand::Value(Value(1)))));
        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            coalesced_pair(&candidates).is_none(),
            "exit path reading the old phi must not coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_switch_terminator_old_phi_use() {
        // State-machine shape: `state_next = ...; switch (state) ...`.
        // The SOURCE block's terminator reads the old phi after the
        // destructive update; the latch is a real successor (case 0) so
        // the candidate would otherwise be well-formed.  The
        // read-by-terminator check must veto it.
        let mut func = split_latch_func(Terminator::Return(Some(Operand::Const(IrConst::I32(0)))));
        func.blocks[1].terminator = Terminator::Switch {
            val: Operand::Value(Value(1)),
            cases: vec![(0, BlockId(2))],
            default: BlockId(3),
            ty: IrType::I32,
        };
        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            coalesced_pair(&candidates).is_none(),
            "switch reading the old phi must not coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_cast_to_gep_chain_escaping_source_block() {
        // A Cast of the phi feeding a GEP keeps the address dependency:
        // the backend folds cast+GEP chains into the SIB operand of the
        // consuming access, re-reading the phi's register there. Escaping
        // the source block must veto coalescing even though the GEP's
        // operand is the cast dest, not the phi itself.
        let mut func = split_latch_func(Terminator::Return(Some(Operand::Const(IrConst::I32(0)))));
        func.blocks[1].instructions.insert(
            0,
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
        );
        func.blocks[1].instructions.insert(
            1,
            Instruction::GetElementPtr {
                dest: Value(6),
                base: Value(2),
                offset: Operand::Value(Value(5)),
                ty: IrType::I32,
            },
        );
        func.blocks[3].instructions = vec![Instruction::Load {
            dest: Value(7),
            ptr: Value(6),
            ty: IrType::I32,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        }];
        func.next_value_id = 8;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            coalesced_pair(&candidates).is_none(),
            "cast->GEP address chain escaping the source block must not coalesce: {candidates:?}"
        );
    }

    #[test]
    fn rejects_src_use_via_second_backedge_copy() {
        // Two backedges to the same header.  v2 is defined in block 1 and
        // used in block 4.  The CLEAN path 1 -> 2(eliminated latch copy) ->
        // 0 -> 4 is safe, but the loop can also take 4 -> 5 (second latch
        // re-defines v1!) -> 0 -> 4: on that pass block 4 reads the shared
        // home AFTER the second backedge copy clobbered it, without a fresh
        // v2 definition in between.  The dirty-path search must veto the
        // coalescing even though a clean path exists.
        let mut func = IrFunction::new("two_backedges".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            // 7: preheader — init copies live here and execute ONCE; the
            //    header (0) must not re-define the phi dest on every pass.
            block(
                7,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I32(0)),
                    },
                    Instruction::Copy {
                        dest: Value(9),
                        src: Operand::Const(IrConst::I32(0)),
                    },
                ],
                Terminator::Branch(BlockId(0)),
            ),
            // 0: header.
            block(
                0,
                Vec::new(),
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(4),
                },
            ),
            block(
                1,
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            // 2: first backedge latch — the candidate copy.
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }],
                Terminator::Branch(BlockId(0)),
            ),
            block(3, Vec::new(), Terminator::Branch(BlockId(4))),
            // 4: uses v2.
            block(
                4,
                vec![Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(3)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(6)),
                    true_label: BlockId(5),
                    false_label: BlockId(6),
                },
            ),
            // 5: second backedge latch — re-defines the phi dest.
            block(
                5,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(9)),
                }],
                Terminator::Branch(BlockId(0)),
            ),
            block(
                6,
                Vec::new(),
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        func.next_value_id = 10;

        let liveness = compute_live_intervals(&func);
        let candidates = detect_phi_coalesce_groups(&func, &liveness);
        assert!(
            !candidates
                .iter()
                .any(|c| c.phi_dest == 1 && c.backedge_src == 2),
            "second backedge copy of the phi must veto coalescing: {candidates:?}"
        );
    }
}

#[cfg(test)]
mod map_collector_tests {
    use super::*;
    use crate::ir::instruction::{Instruction, Terminator};
    use crate::ir::intrinsics::IntrinsicOp as O;
    use crate::ir::reexports::{BasicBlock, BlockId, Value};

    fn intrinsic(op: O, dest: u32, args: Vec<Operand>) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op,
            dest_ptr: None,
            args,
        }
    }

    fn store(op: O, src: u32, ptr: Option<u32>) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: ptr.map(Value),
            args: vec![Operand::Value(Value(src))],
        }
    }

    fn func(insts: Vec<Instruction>, term: Terminator) -> IrFunction {
        let mut f = IrFunction::new("map_collector".to_string(), IrType::Void, vec![], false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }];
        f
    }

    #[test]
    fn strict_reciprocal_denominators_are_128_bit_in_every_home_collector() {
        for (producer, expected) in [
            (O::VecAddI32x4, true),
            (O::VecBroadcastI32x4, true),
            (O::VecMulI32x4, true),
            (O::VecAddI32x8, false),
            (O::VecBroadcastI32x8, false),
            (O::VecMulI32x8, false),
        ] {
            let arguments = if matches!(producer, O::VecBroadcastI32x4 | O::VecBroadcastI32x8) {
                vec![Operand::Const(IrConst::I32(7))]
            } else {
                vec![Operand::Value(Value(9)), Operand::Value(Value(9))]
            };
            let f = func(
                vec![
                    intrinsic(producer, 1, arguments),
                    intrinsic(
                        O::StrictRecipMulAddPackedF64x4,
                        2,
                        vec![
                            Operand::Const(IrConst::F64(0.0)),
                            Operand::Value(Value(1)),
                            Operand::Value(Value(100)),
                            Operand::Const(IrConst::I64(0)),
                            Operand::Value(Value(101)),
                        ],
                    ),
                ],
                Terminator::Return(None),
            );
            let sets = [
                collect_x86_map_broadcast_values(&f),
                collect_x86_map_intermediate_values(&f),
            ];
            let admitted = sets.iter().any(|set| set.contains(&1));
            assert_eq!(admitted, expected, "{producer:?}: {sets:?}");
            if matches!(producer, O::VecBroadcastI32x4 | O::VecBroadcastI32x8) {
                assert_eq!(sets[0].contains(&1), expected);
            } else {
                assert_eq!(sets[1].contains(&1), expected);
                // Reduction homes deliberately require a Copy web. A direct
                // transient is not an accumulator, even at the correct width.
                assert!(!collect_x86_reduction_vector_values(&f).contains(&1));
                let mut instructions = f.blocks[0].instructions.clone();
                instructions.insert(
                    1,
                    Instruction::Copy {
                        dest: Value(3),
                        src: Operand::Value(Value(1)),
                    },
                );
                let Instruction::Intrinsic { args, .. } = &mut instructions[2] else {
                    unreachable!();
                };
                args[1] = Operand::Value(Value(3));
                let web = func(instructions, Terminator::Return(None));
                let reduction = collect_x86_reduction_vector_values(&web);
                for value in [1, 3] {
                    assert_eq!(
                        reduction.contains(&value),
                        expected,
                        "{producer:?}: {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn admits_multi_use_load_min_and_cmp_webs() {
        // Clamp body: %1 = load; %2 = min(%1, %1); %3 = cmp(%1, %1);
        // %4 = blendv(%3, %2, %2); %5 = store(%4).
        let f = func(
            vec![
                intrinsic(O::VecLoadF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
                intrinsic(
                    O::VecCmpF32x8,
                    3,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
                intrinsic(
                    O::VecBlendvF32x8,
                    4,
                    vec![
                        Operand::Value(Value(3)),
                        Operand::Value(Value(2)),
                        Operand::Value(Value(2)),
                    ],
                ),
                store(O::VecStoreF32x8, 4, Some(101)),
            ],
            Terminator::Return(None),
        );
        let set = collect_x86_map_intermediate_values(&f);
        assert_eq!(set.len(), 4, "load/min/cmp/blendv all admitted: {set:?}");
        for v in [1, 2, 3, 4] {
            assert!(set.contains(&v), "missing value {v}");
        }
    }

    #[test]
    fn rejects_terminator_consumers() {
        // The load feeds a CondBranch in addition to the min chain: a
        // terminator position that reads the value as a scalar.
        let f = func(
            vec![
                intrinsic(O::VecLoadF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
                store(O::VecStoreF32x8, 2, Some(101)),
            ],
            Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(0),
                false_label: BlockId(0),
            },
        );
        let set = collect_x86_map_intermediate_values(&f);
        assert!(
            !set.contains(&1),
            "terminator consumer must strand the load"
        );
        assert!(set.contains(&2), "min still admissible");
    }

    #[test]
    fn admits_register_valued_vector_return() {
        // `return a + b;` as the SIMD lowering shapes it: the packed result
        // is the Return operand (`emit_return_impl` resolves it through the
        // register home), so the value keeps its home.
        let f = func(
            vec![
                intrinsic(O::VecLoadF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        );
        let set = collect_x86_map_intermediate_values(&f);
        assert!(set.contains(&1), "load admitted: {set:?}");
        assert!(set.contains(&2), "returned result keeps its home: {set:?}");
    }

    #[test]
    fn rejects_address_side_consumers() {
        // The min result is used as the VecStore DESTINATION POINTER
        // (address-side use) — must strand it even though the op is listed.
        let f = func(
            vec![
                intrinsic(O::VecLoadF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
                store(O::VecStoreF32x8, 9, Some(2)),
            ],
            Terminator::Return(None),
        );
        let set = collect_x86_map_intermediate_values(&f);
        assert!(!set.contains(&2), "dest_ptr use must strand the value");
    }

    #[test]
    fn rejects_unlisted_consumers() {
        // The min feeds a load address (VecLoadF32x8 is not a listed
        // consumer of class 1): fail-closed.
        let f = func(
            vec![
                intrinsic(O::VecLoadF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                ),
                intrinsic(O::VecLoadF32x8, 3, vec![Operand::Value(Value(2))]),
                store(O::VecStoreF32x8, 3, Some(101)),
            ],
            Terminator::Return(None),
        );
        let set = collect_x86_map_intermediate_values(&f);
        assert!(
            !set.contains(&2),
            "load-address consumer must strand the min"
        );
    }

    #[test]
    fn broadcast_admits_min_cmp_blend_consumers() {
        // Loop-invariant broadcast feeding the 16 new ops (follow-up #1):
        // the widened legal_consumer must admit each class.
        let f = func(
            vec![
                intrinsic(O::VecBroadcastF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(9)), Operand::Value(Value(1))],
                ),
                intrinsic(
                    O::VecCmpF32x8,
                    3,
                    vec![Operand::Value(Value(9)), Operand::Value(Value(1))],
                ),
                intrinsic(
                    O::VecBlendvF32x8,
                    4,
                    vec![
                        Operand::Value(Value(3)),
                        Operand::Value(Value(2)),
                        Operand::Value(Value(1)),
                    ],
                ),
                store(O::VecStoreF32x8, 4, Some(101)),
            ],
            Terminator::Return(None),
        );
        let set = collect_x86_map_broadcast_values(&f);
        assert!(
            set.contains(&1),
            "broadcast feeding min/cmp/blendv admitted: {set:?}"
        );
    }

    #[test]
    fn broadcast_still_stranded_by_unlisted_consumer() {
        // Same web, but the broadcast also feeds VecWidenMaskedAddI32x4ToI64x2
        // (never a packed FP consumer): fail-closed.
        let f = func(
            vec![
                intrinsic(O::VecBroadcastF32x8, 1, vec![Operand::Value(Value(100))]),
                intrinsic(
                    O::VecMinF32x8,
                    2,
                    vec![Operand::Value(Value(9)), Operand::Value(Value(1))],
                ),
                intrinsic(
                    O::VecWidenMaskedAddI32x4ToI64x2,
                    3,
                    vec![
                        Operand::Value(Value(1)),
                        Operand::Value(Value(1)),
                        Operand::Value(Value(1)),
                        Operand::Value(Value(1)),
                    ],
                ),
                store(O::VecStoreF32x8, 2, Some(101)),
            ],
            Terminator::Return(None),
        );
        let set = collect_x86_map_broadcast_values(&f);
        assert!(
            !set.contains(&1),
            "unlisted consumer must strand the broadcast"
        );
    }
}

#[cfg(test)]
mod allocation_kernel_tests {
    use super::*;
    use crate::ir::analysis;
    use crate::ir::intrinsics::IntrinsicOp;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst, Value};

    fn iv(value_id: u32, start: u32, end: u32) -> LiveInterval {
        LiveInterval {
            value_id,
            start,
            end,
        }
    }

    fn liveness_fixture(
        intervals: Vec<LiveInterval>,
        segments: Vec<LiveInterval>,
    ) -> LivenessResult {
        let mut func = IrFunction::new("kernel_fixture".to_string(), IrType::Void, vec![], false);
        func.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: Vec::new(),
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        let mut liveness = compute_live_intervals(&func);
        liveness.intervals = intervals;
        liveness.segments = segments;
        liveness
    }

    /// COLD-CALL XMM SPLIT save set: an XMM home is saved around a cold
    /// call exactly when a value in it is live ACROSS the call -- never the
    /// register receiving the call's result, even when a merged copy web
    /// counts a member live into the call block (the nbody/`norm` shape:
    /// restoring there overwrote the slow-path `sqrt` result).
    #[test]
    fn cold_call_xmm_saves_skip_the_result_register() {
        let iv = |value_id, start, end| LiveInterval {
            value_id,
            start,
            end,
        };
        // v23: loop accumulator live across the call at 24 (xmm2 = id 20).
        // v17: phi web member counted live into the slow block from its
        //      start (= the call point); the call result v26 shares xmm4.
        let mut liveness = liveness_fixture(
            vec![
                iv(23, 5, 27),
                iv(17, 16, 26),
                iv(26, 24, 25),
                iv(30, 24, 29),
            ],
            vec![
                iv(23, 5, 27),
                iv(17, 24, 26),
                iv(26, 24, 25),
                iv(30, 24, 29),
            ],
        );
        liveness.call_points = vec![24];
        liveness.cold_call_points = vec![24];
        liveness.cold_call_results = vec![Some(26)];
        let iv_map = interval_map(&liveness);
        let mut assignments: FxHashMap<u32, PhysReg> = FxHashMap::default();
        assignments.insert(23, PhysReg(20));
        assignments.insert(17, PhysReg(22));
        assignments.insert(26, PhysReg(22));
        let mut spans: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        add_cold_call_xmm_saves(&assignments, &liveness, &iv_map, true, &mut spans);
        assert_eq!(spans.get(&20), Some(&vec![(24, 24)]), "accumulator saved");
        assert!(
            !spans.contains_key(&22),
            "result register never restored over"
        );

        // v30: a result live after the call in its own register is born there.
        liveness.cold_call_results = vec![Some(30)];
        assignments.clear();
        assignments.insert(30, PhysReg(25));
        spans.clear();
        add_cold_call_xmm_saves(&assignments, &liveness, &iv_map, true, &mut spans);
        assert!(spans.is_empty());

        // Disabled (i686 / CCC_NO_COLD_CALL_SPLIT): nothing recorded.
        assignments.insert(23, PhysReg(20));
        add_cold_call_xmm_saves(&assignments, &liveness, &iv_map, false, &mut spans);
        assert!(spans.is_empty());
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    /// Regression (kernel 6.18.52 workqueue.c llc_populate_cpu_shard_id):
    /// a coalesced induction chain (v107→v109→v103) homed slot-less in one
    /// register must be published as ONE same-value class in
    /// RegAllocResult::phi_chain, so the emitter's alias-aware freshness can
    /// accept a fresh sibling as proof the home is readable after an
    /// in-place chain update evicted the older member ids.
    #[test]
    fn phi_chain_published_for_applied_coalesce() {
        // Induction chain with a backedge copy: entry v1 = 0, v7 = v1;
        // head v9 = v7 (latch-carried); latch v10 = v9 + 1, v7 = v10.
        // Applied phi pairs (v7<-v10-class links) must be published as one
        // same-value class in RegAllocResult::phi_chain, each member
        // resolving to a representative that does not chain further.
        let mut func = IrFunction::new("chain".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I32(0)),
                    },
                    Instruction::Copy {
                        dest: Value(7),
                        src: Operand::Value(Value(1)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::Copy {
                    dest: Value(9),
                    src: Operand::Value(Value(7)),
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![
                    Instruction::BinOp {
                        dest: Value(10),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(9)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Copy {
                        dest: Value(7),
                        src: Operand::Value(Value(10)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![Instruction::Copy {
                    dest: Value(11),
                    src: Operand::Value(Value(9)),
                }],
                Terminator::Branch(BlockId(4)),
            ),
            block(
                4,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(11)))),
            ),
        ];
        func.next_value_id = 12;
        let config = RegAllocConfig {
            ipa_regs: Vec::new(),
            available_regs: vec![PhysReg(1), PhysReg(2), PhysReg(3), PhysReg(4), PhysReg(5)],
            accumulator_policy: AccumulatorPolicy {
                operand_order: AccumulatorOperandOrder::LhsFirst,
                return_consumes_accumulator: false,
            },
            caller_saved_regs: vec![],
            call_arg_regs: vec![],
            indirect_target_regs: vec![],
            allow_inline_asm_regalloc: false,
            leaf_caller_saved_homes: false,
            xmm_regs: Vec::new(),
            never_materialized: FxHashSet::default(),
            folded_index_uses: FxHashMap::default(),
            fold_base_regs: Vec::new(),
            fold_readmit: FxHashSet::default(),
            reg_hints: FxHashMap::default(),
            ra_config: Arc::new(RaConfig::default()),
        };
        let result = allocate_registers(&func, &config);
        // POSITIVE evidence (audit F6.2): an empty map would make the
        // structural loop below vacuous and silently re-open the RA-3 gap
        // (emitter assumes the chain is published; coalescer stops writing
        // it).  The applied backedge phi pair v7<-v10 MUST be published.
        assert!(
            !result.phi_chain.is_empty(),
            "RA-3 contract: applied coalesce classes must be published, got none"
        );
        // Map shape: member id -> representative id; representatives are
        // never keys themselves (v7 is the class root here).  The applied
        // backedge phi publishes v10 -> v7.
        assert_eq!(
            result.phi_chain.get(&10).copied(),
            Some(7),
            "the applied phi pair must publish v10 into v7's class: {:?}",
            result.phi_chain
        );
        let rep = |v: u32| result.phi_chain.get(&v).copied().unwrap_or(v);
        assert_eq!(rep(7), rep(10), "v7 and v10 denote one value");
        assert_eq!(rep(7), 7, "representative is the minimum id");
        // The slot web may coalesce further members (the observed exit-copy
        // v11 -> v1); every published link must still resolve to a root —
        // checked by the structural loop below.
        // Non-key invariant: representatives are never members of their own
        // class map (v1 roots {11}, v7 roots {10}).
        assert!(
            !result.phi_chain.contains_key(&7) && !result.phi_chain.contains_key(&1),
            "representatives must not appear as keys: {:?}",
            result.phi_chain
        );
        for (&member, &r) in &result.phi_chain {
            assert_eq!(
                rep(r),
                r,
                "representative must not chain further: v{member} -> v{r}"
            );
        }
        // Determinism: identical input → identical published chain.
        let result2 = allocate_registers(&func, &config);
        assert_eq!(
            result.phi_chain, result2.phi_chain,
            "phi_chain publication must be deterministic"
        );
    }

    fn quadratic_overlaps(spans: &[(u8, u32, u32, u32)]) -> Vec<(u8, u32, u32, u32, u32)> {
        let mut out = Vec::new();
        for (i, &(reg_a, start_a, end_a, class_a)) in spans.iter().enumerate() {
            for &(reg_b, start_b, end_b, class_b) in spans.iter().skip(i + 1) {
                if reg_a != reg_b || class_a == class_b {
                    continue;
                }
                if intervals_overlap((start_a, end_a), (start_b, end_b)) {
                    out.push((
                        reg_a,
                        class_a.min(class_b),
                        class_a.max(class_b),
                        start_a.max(start_b),
                        end_a.min(end_b),
                    ));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    #[test]
    fn overlap_kernel_reports_all_pairwise_class_conflicts() {
        // A[0,100], B[1,20], C[2,30] on one register: the previous-max-end
        // scan missed B↔C because C starts before A's end.
        let spans = vec![(1u8, 0u32, 100u32, 10u32), (1, 1, 20, 11), (1, 2, 30, 12)];
        let got = overlapping_class_spans(spans.clone());
        let expect = quadratic_overlaps(&spans);
        assert_eq!(got, expect);
        assert!(
            got.iter().any(|&(_, a, b, _, _)| a == 11 && b == 12),
            "B↔C conflict must be reported: {got:?}"
        );
        assert!(got.iter().any(|&(_, a, b, _, _)| a == 10 && b == 11));
        assert!(got.iter().any(|&(_, a, b, _, _)| a == 10 && b == 12));
    }

    #[test]
    fn overlap_kernel_keeps_zero_length_and_half_open_boundaries() {
        assert!(intervals_overlap((0, 10), (5, 5)));
        assert!(!intervals_overlap((0, 5), (5, 9)));
        let zero_inside = overlapping_class_spans(vec![(3, 0, 10, 1), (3, 5, 5, 2)]);
        assert_eq!(zero_inside, vec![(3, 1, 2, 5, 5)]);
        let adjacent = overlapping_class_spans(vec![(3, 0, 5, 1), (3, 5, 9, 2)]);
        assert!(adjacent.is_empty(), "{adjacent:?}");
    }

    #[test]
    fn overlap_kernel_matches_quadratic_oracle() {
        let mut seed: u64 = 0xC0FFEE;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            seed
        };
        for _ in 0..200 {
            let n = (next() % 8) as usize;
            let mut spans = Vec::with_capacity(n);
            for _ in 0..n {
                let reg = (next() % 3) as u8;
                let class = (next() % 5) as u32;
                let start = (next() % 16) as u32;
                let span = (next() % 8) as u32;
                spans.push((reg, start, start + span, class));
            }
            let got = overlapping_class_spans(spans.clone());
            let expect = quadratic_overlaps(&spans);
            assert_eq!(got, expect, "spans={spans:?}");
        }
    }

    #[test]
    fn flatten_resolves_indirect_class_parents() {
        let mut parent = FxHashMap::default();
        parent.insert(1, 2);
        parent.insert(2, 3);
        parent.insert(3, 3);
        flatten_allocation_classes(&mut parent);
        assert_eq!(parent.get(&1).copied(), Some(3));
        assert_eq!(parent.get(&2).copied(), Some(3));
        assert_eq!(allocation_class_root(&parent, 1), 3);
        assert_eq!(allocation_class_root(&parent, 99), 99);
    }

    #[test]
    fn interval_map_keeps_complete_envelope() {
        let liveness = liveness_fixture(vec![iv(7, 2, 4), iv(7, 10, 18), iv(7, 0, 3)], Vec::new());
        let map = interval_map(&liveness);
        assert_eq!(map.get(&7).copied(), Some((0, 18)));
    }

    #[test]
    fn collect_gpr_requires_eligibility_and_sorts() {
        let liveness = liveness_fixture(
            vec![iv(1, 8, 12), iv(2, 0, 4), iv(3, 1, 20), iv(4, 5, 6)],
            Vec::new(),
        );
        let mut eligible = FxHashSet::default();
        eligible.insert(1);
        eligible.insert(2);
        eligible.insert(4);
        let mut merged_of = FxHashMap::default();
        merged_of.insert(
            3,
            LiveInterval {
                value_id: 3,
                start: 1,
                end: 20,
            },
        );
        merged_of.insert(
            1,
            LiveInterval {
                value_id: 1,
                start: 8,
                end: 40,
            },
        );
        let mut member_of = FxHashMap::default();
        member_of.insert(4, 1);
        let scan = collect_gpr_scan_intervals(&liveness, &eligible, &merged_of, &member_of);
        let ids: Vec<u32> = scan.iter().map(|i| i.value_id).collect();
        assert_eq!(
            ids,
            vec![2, 1],
            "ineligible merged 3 and member 4 must drop"
        );
        assert_eq!(scan[1].end, 40);
    }

    #[test]
    fn skip_birth_consumes_every_duplicate_birth_hazard() {
        let interval = iv(1, 5, 10);
        assert!(!overlaps_inclusive_skip_birth(&interval, &[5, 5, 5]));
        assert!(overlaps_inclusive_skip_birth(&interval, &[5, 5, 5, 8]));
        assert!(overlaps_inclusive_skip_birth(&interval, &[5, 10]));
        assert!(!overlaps_inclusive_skip_birth(&interval, &[4, 5]));
    }

    #[test]
    fn owned_segments_keep_unsegmented_web_members() {
        let liveness = liveness_fixture(
            vec![iv(1, 0, 10), iv(2, 20, 30)],
            vec![iv(1, 0, 4), iv(1, 8, 10)],
        );
        let mut member_of = FxHashMap::default();
        member_of.insert(2, 1);
        let owned = owned_live_segments(&liveness, &member_of);
        let pieces = owned.get(&1).expect("owner coverage");
        assert!(
            pieces.iter().any(|&span| span == (20, 30)),
            "unsegmented member 2 must contribute: {pieces:?}"
        );
        assert!(
            pieces
                .iter()
                .any(|&span| span == (0, 4) || span == (0, 10) || span == (8, 10))
        );
    }

    fn divrem_func(insts: Vec<Instruction>) -> IrFunction {
        let mut func = IrFunction::new("divrem_kernel".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![block(0, insts, Terminator::Return(None))];
        func.next_value_id = 20;
        func
    }

    fn binop(dest: u32, op: IrBinOp, ty: IrType, lhs: Operand, rhs: Operand) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            ty,
            lhs,
            rhs,
        }
    }

    #[test]
    fn divrem_pairs_matching_operands_and_width() {
        let func = divrem_func(vec![
            binop(
                2,
                IrBinOp::UDiv,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
            binop(
                3,
                IrBinOp::URem,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
        ]);
        let pairs = compute_i686_divrem_pairs(&func, DivRemTarget::I686);
        assert!(
            pairs.tail_dests.contains(&3),
            "urem must be the tail: {:?}",
            pairs.tail_dests
        );
        assert_eq!(pairs.head_partners.get(&2).copied(), Some((3, false)));
    }

    #[test]
    fn divrem_rejects_width_mismatch() {
        let func = divrem_func(vec![
            binop(
                2,
                IrBinOp::UDiv,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
            binop(
                3,
                IrBinOp::URem,
                IrType::I16,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
        ]);
        let pairs = compute_i686_divrem_pairs(&func, DivRemTarget::I686);
        assert!(pairs.tail_dests.is_empty(), "{:?}", pairs.tail_dests);
    }

    #[test]
    fn divrem_rejects_redefined_operand_stamp() {
        let func = divrem_func(vec![
            Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::I32(8)),
            },
            binop(
                2,
                IrBinOp::UDiv,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
            Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::I32(9)),
            },
            binop(
                3,
                IrBinOp::URem,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
        ]);
        let pairs = compute_i686_divrem_pairs(&func, DivRemTarget::I686);
        assert!(
            pairs.tail_dests.is_empty(),
            "redef between div and rem must not pair: {:?}",
            pairs.tail_dests
        );
    }

    #[test]
    fn divrem_rejects_exceptional_barrier() {
        let func = divrem_func(vec![
            binop(
                2,
                IrBinOp::UDiv,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::BuiltinSetjmp,
                dest_ptr: None,
                args: vec![],
            },
            binop(
                3,
                IrBinOp::URem,
                IrType::I32,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I32(3)),
            ),
        ]);
        let pairs = compute_i686_divrem_pairs(&func, DivRemTarget::I686);
        assert!(
            pairs.tail_dests.is_empty(),
            "setjmp between div and rem must not pair: {:?}",
            pairs.tail_dests
        );
    }

    /// Parameters may take caller-saved homes in an %rdx-clobbering body only
    /// when every clobber is a narrow division/remainder/multiply-high (or a
    /// switch dispatch) and the body holds no wide value; both kill switches
    /// restore the whole-function exclusion.
    #[test]
    fn rdx_clobber_classes_admitted_for_param_homes() {
        let narrow = |op| {
            binop(
                2,
                op,
                IrType::I64,
                Operand::Value(Value(1)),
                Operand::Const(IrConst::I64(3)),
            )
        };
        let cfg = RaConfig::default();
        for op in [
            IrBinOp::UDiv,
            IrBinOp::SDiv,
            IrBinOp::URem,
            IrBinOp::SRem,
            IrBinOp::UMulHigh,
            IrBinOp::SMulHigh,
        ] {
            assert!(
                x86_rdx_clobbers_allow_param_homes(&divrem_func(vec![narrow(op)]), &cfg),
                "{op:?}"
            );
        }
        let mut switch_body = divrem_func(vec![narrow(IrBinOp::UDiv)]);
        switch_body.blocks[0].terminator = Terminator::Switch {
            val: Operand::Value(Value(2)),
            cases: vec![(0, BlockId(0))],
            default: BlockId(0),
            ty: IrType::I64,
        };
        assert!(x86_rdx_clobbers_allow_param_homes(&switch_body, &cfg));
        let wide_div = binop(
            2,
            IrBinOp::UDiv,
            IrType::U128,
            Operand::Value(Value(1)),
            Operand::Const(IrConst::I128(3)),
        );
        assert!(!x86_rdx_clobbers_allow_param_homes(
            &divrem_func(vec![wide_div]),
            &cfg
        ));
        let wide_copy = Instruction::Copy {
            dest: Value(3),
            src: Operand::Const(IrConst::I128(1)),
        };
        assert!(!x86_rdx_clobbers_allow_param_homes(
            &divrem_func(vec![narrow(IrBinOp::UDiv), wide_copy]),
            &cfg
        ));
        let rdtsc = Instruction::Intrinsic {
            dest: Some(Value(3)),
            op: IntrinsicOp::Rdtsc,
            dest_ptr: None,
            args: vec![],
        };
        assert!(!x86_rdx_clobbers_allow_param_homes(
            &divrem_func(vec![narrow(IrBinOp::UDiv), rdtsc]),
            &cfg
        ));
        let body = divrem_func(vec![narrow(IrBinOp::UDiv)]);
        for off in [
            RaConfig {
                no_rdx_param_homes: true,
                ..RaConfig::default()
            },
            RaConfig {
                no_rdx_hazard: true,
                ..RaConfig::default()
            },
        ] {
            assert!(!x86_rdx_clobbers_allow_param_homes(&body, &off));
        }
    }

    #[test]
    fn wide_ops_see_i128_constants_without_typed_def() {
        let mut func = IrFunction::new("wide_const".to_string(), IrType::Void, vec![], false);
        func.blocks = vec![block(
            0,
            vec![Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::I128(1)),
            }],
            Terminator::Return(None),
        )];
        func.next_value_id = 2;
        assert!(x86_body_has_wide_ops(&func));
    }

    #[test]
    fn propagate_restrictions_to_web_owners() {
        let mut restricted = FxHashSet::default();
        restricted.insert(4);
        let mut member_of = FxHashMap::default();
        member_of.insert(4, 1);
        propagate_member_restrictions(&mut restricted, &member_of);
        assert!(restricted.contains(&1));
        assert!(restricted.contains(&4));
    }

    #[test]
    #[should_panic(expected = "invalid register-allocation range")]
    fn overlap_kernel_rejects_reversed_range() {
        let _ = overlapping_class_spans(vec![(1, 10, 3, 7)]);
    }

    #[test]
    fn call_spanning_falls_back_to_fat_interval() {
        let mut liveness = liveness_fixture(vec![iv(1, 0, 20)], Vec::new());
        liveness.call_points = vec![10];
        let iv_map = interval_map(&liveness);
        let spanning = collect_call_spanning_owners(
            &liveness,
            &iv_map,
            &FxHashMap::default(),
            &liveness.call_points,
        );
        assert!(spanning.contains(&1), "{spanning:?}");
    }

    #[test]
    fn call_spanning_respects_segment_gaps() {
        let mut liveness = liveness_fixture(vec![iv(1, 0, 20)], vec![iv(1, 0, 5), iv(1, 15, 20)]);
        liveness.call_points = vec![10];
        let iv_map = interval_map(&liveness);
        let spanning = collect_call_spanning_owners(
            &liveness,
            &iv_map,
            &FxHashMap::default(),
            &liveness.call_points,
        );
        assert!(
            spanning.is_empty(),
            "call in a segment gap must not span: {spanning:?}"
        );
    }

    #[test]
    fn source_home_rejects_copy_block_redef() {
        // After the latch copy, dest is overwritten and `src` is still
        // read. Sharing a home would return the clobbered dest.
        let mut func = IrFunction::new("redef".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(1)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(2)),
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I32(9)),
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ];
        func.next_value_id = 3;
        let liveness = compute_live_intervals(&func);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 1,
            source_block_idx: 0,
            source_def_idx: 0,
            copy_idx: 0,
        };
        assert!(!source_home_survives_dest_redefs(
            &func, &liveness, &succs, &cand
        ));
    }

    fn add(dest: u32, lhs: u32, k: i64) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(IrConst::I64(k)),
            ty: IrType::I64,
        }
    }

    fn copy(dest: u32, src: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
        }
    }

    fn store(val: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty,
            seg_override: Default::default(),
            volatile: false,
        }
    }

    /// The if/else latch of hot_web_loop: `x = old + 1; if (c) { x' = x + 5;
    /// use(x + 7); old = x' } else old = x`.  `rare_tail` is the rare arm
    /// (block 3) after its sibling definition `v3 = v2 + 5`.
    fn if_else_latch(rare_tail: Vec<Instruction>) -> IrFunction {
        let mut func = IrFunction::new("latch".to_string(), IrType::I64, vec![], false);
        let mut rare = vec![add(3, 2, 5)];
        rare.extend(rare_tail);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                    Instruction::GlobalAddr {
                        dest: Value(9),
                        name: "g".to_string(),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![add(2, 1, 1)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(2, vec![copy(1, 2)], Terminator::Branch(BlockId(4))),
            block(3, rare, Terminator::Branch(BlockId(4))),
            block(
                4,
                Vec::new(),
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(5),
                },
            ),
            block(
                5,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 10;
        func
    }

    const COMMON_ARM: PhiCoalesceCandidate = PhiCoalesceCandidate {
        phi_dest: 1,
        backedge_src: 2,
        block_idx: 2,
        source_block_idx: 1,
        source_def_idx: 0,
        copy_idx: 0,
    };

    /// A sibling source coalesced into the same home writes it at its OWN
    /// definition: a read of the common-arm source after that point (but
    /// before the rare arm's copy) would see `x + 5`, not `x`.  Only
    /// dest redefinitions used to dirty the walk, so this was accepted.
    #[test]
    fn source_home_rejects_a_read_after_a_sibling_source_definition() {
        let func = if_else_latch(vec![add(4, 2, 7), store(4, 9, IrType::I64), copy(1, 3)]);
        let liveness = compute_live_intervals(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &analysis::build_label_map(&func));
        assert!(!source_home_survives_dest_redefs(
            &func,
            &liveness,
            &succs,
            &COMMON_ARM
        ));
        // Reading the source BEFORE the sibling definition is clean.
        let mut ok = if_else_latch(vec![copy(1, 3)]);
        ok.blocks[3].instructions.insert(0, add(4, 2, 7));
        ok.blocks[3]
            .instructions
            .insert(1, store(4, 9, IrType::I64));
        let liveness = compute_live_intervals(&ok);
        let (_preds, succs) = analysis::build_cfg(&ok, &analysis::build_label_map(&ok));
        assert!(source_home_survives_dest_redefs(
            &ok,
            &liveness,
            &succs,
            &COMMON_ARM
        ));
    }

    /// The block-level use-path test reads instruction order inside a use
    /// block that redefines the dest: reads before its first redefinition
    /// are clean, reads after it (or in its terminator) are not.
    #[test]
    fn use_path_respects_instruction_order_in_a_redefining_use_block() {
        let func = if_else_latch(vec![copy(1, 3)]); // rare arm reads v2 first
        let (_preds, succs) = analysis::build_cfg(&func, &analysis::build_label_map(&func));
        assert!(src_use_path_clear_of_dest_redefs(
            &func, &succs, 3, 1, 2, 1, 2
        ));
        let late = if_else_latch(vec![copy(1, 3), add(4, 2, 7), store(4, 9, IrType::I64)]);
        let (_preds, succs) = analysis::build_cfg(&late, &analysis::build_label_map(&late));
        assert!(!src_use_path_clear_of_dest_redefs(
            &late, &succs, 3, 1, 2, 1, 2
        ));
        let mut term = if_else_latch(vec![copy(1, 3)]);
        term.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(2))));
        let (_preds, succs) = analysis::build_cfg(&term, &analysis::build_label_map(&term));
        assert!(!src_use_path_clear_of_dest_redefs(
            &term, &succs, 3, 1, 2, 1, 2
        ));
    }

    /// gzip_crc32: a cast of the source (`v3 = trunc v2`) is an ordinary
    /// holder for the apply phase's home-conflict test, not a pair sibling:
    /// its definition must not dirty the walk and veto the coalescing.
    #[test]
    fn source_home_ignores_a_cast_of_the_source() {
        let mut func = IrFunction::new("cast".to_string(), IrType::I64, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                    Instruction::GlobalAddr {
                        dest: Value(9),
                        name: "g".to_string(),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    add(2, 1, 1),
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(2)),
                        from_ty: IrType::I64,
                        to_ty: IrType::U8,
                    },
                    store(3, 9, IrType::U8),
                    copy(1, 2),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
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
        func.next_value_id = 10;
        let liveness = compute_live_intervals(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &analysis::build_label_map(&func));
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 1,
            source_block_idx: 1,
            source_def_idx: 0,
            copy_idx: 3,
        };
        assert!(source_home_survives_dest_redefs(
            &func, &liveness, &succs, &cand
        ));
    }

    #[test]
    fn source_home_accepts_clean_same_block_window() {
        let mut func = IrFunction::new("clean".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![block(
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
            Terminator::Return(Some(Operand::Value(Value(1)))),
        )];
        func.next_value_id = 3;
        let liveness = compute_live_intervals(&func);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 1,
            backedge_src: 2,
            block_idx: 0,
            source_block_idx: 0,
            source_def_idx: 1,
            copy_idx: 2,
        };
        assert!(source_home_survives_dest_redefs(
            &func, &liveness, &succs, &cand
        ));
    }

    #[test]
    fn coverage_overlap_is_linear_and_half_open() {
        assert!(sorted_coverage_overlaps(&[(0, 10)], &[(5, 12)]));
        assert!(!sorted_coverage_overlaps(&[(0, 5)], &[(5, 9)]));
        assert!(!sorted_coverage_overlaps(&[(0, 4), (10, 12)], &[(4, 10)]));
        assert!(sorted_coverage_overlaps(&[(0, 4), (8, 12)], &[(9, 11)]));
    }

    #[test]
    fn phi_web_rejects_overlapping_segments() {
        assert!(sorted_coverage_overlaps(&[(0, 20)], &[(5, 15)]));
        let left = coverage_of_value(
            1,
            &{
                let mut m = FxHashMap::default();
                m.insert(1, vec![(0, 10), (20, 30)]);
                m
            },
            &FxHashMap::default(),
        );
        let right = coverage_of_value(
            2,
            &{
                let mut m = FxHashMap::default();
                m.insert(2, vec![(10, 20)]);
                m
            },
            &FxHashMap::default(),
        );
        assert!(
            !sorted_coverage_overlaps(&left, &right),
            "exclusive arms must not overlap: {left:?} {right:?}"
        );
    }

    #[test]
    fn vecreg_rejects_unknown_dest_ptr_writer() {
        let mut func = IrFunction::new("unknown_writer".to_string(), IrType::Void, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::I8,
                    size: 16,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Intrinsic {
                    dest: None,
                    dest_ptr: Some(Value(1)),
                    op: IntrinsicOp::Loaddqu,
                    args: vec![],
                },
                Instruction::Intrinsic {
                    dest: None,
                    dest_ptr: Some(Value(1)),
                    op: IntrinsicOp::Pxor128,
                    args: vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                },
                Instruction::Intrinsic {
                    dest: None,
                    dest_ptr: Some(Value(1)),
                    op: IntrinsicOp::Lfence,
                    args: vec![],
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 2;
        let set = collect_vecreg_candidates(&func);
        assert!(set.is_empty(), "unknown writer must poison: {set:?}");
    }

    #[test]
    fn source_home_allows_pointer_increment_latch() {
        // zlib-ng adler32 inner loop: `buf += 8` as
        //   v_next = buf + 8; buf = v_next;
        // plus a sibling latch. Sharing buf with v_next is the in-place add.
        let mut func = IrFunction::new("ptr_latch".to_string(), IrType::Ptr, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Const(IrConst::I64(0)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I64(8)),
                        ty: IrType::I64,
                    },
                    Instruction::Load {
                        dest: Value(4),
                        ptr: Value(2),
                        ty: IrType::U8,
                        volatile: false,
                        seg_override: Default::default(),
                    },
                    Instruction::BinOp {
                        dest: Value(5),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Const(IrConst::I64(8)),
                        ty: IrType::I64,
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: Operand::Value(Value(3)),
                    },
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Value(Value(5)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ];
        func.next_value_id = 6;
        let liveness = compute_live_intervals(&func);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 2,
            backedge_src: 5,
            block_idx: 1,
            source_block_idx: 1,
            source_def_idx: 2,
            copy_idx: 4,
        };
        assert!(
            source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "in-place buf+=8 latch must survive"
        );
    }

    #[test]
    fn vecreg_rejects_non_16_byte_alloca() {
        let mut func = IrFunction::new("wide_slot".to_string(), IrType::Void, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::I8,
                    size: 32,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Intrinsic {
                    dest: None,
                    dest_ptr: Some(Value(1)),
                    op: IntrinsicOp::Loaddqu,
                    args: vec![],
                },
                Instruction::Intrinsic {
                    dest: None,
                    dest_ptr: Some(Value(1)),
                    op: IntrinsicOp::Pxor128,
                    args: vec![Operand::Value(Value(1)), Operand::Value(Value(1))],
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 2;
        let set = collect_vecreg_candidates(&func);
        assert!(set.is_empty(), "32-byte alloca must not promote: {set:?}");
    }

    #[test]
    fn saved_callee_fill_places_the_hottest_web_first() {
        // zstd's decode loop: an early-starting single-use load (pressure 10)
        // and a later per-sequence byte read three times in the loop
        // (pressure 30) both fit only the one free stretch of %rbx. A
        // start-ordered fill gave it to the early value; the hot one wins.
        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        occupied.insert(1, vec![(0, 10), (40, 50)]);
        let candidates = vec![(10, 20, 7, (11, 31)), (30, 5, 9, (25, 30))];
        let placed = fill_saved_callee_stretches(candidates, &mut occupied, &[PhysReg(1)]);
        assert_eq!(placed, vec![(9, PhysReg(1))]);
        assert_eq!(
            occupied.get(&1).cloned().unwrap_or_default(),
            vec![(0, 10), (25, 30), (40, 50)]
        );
    }

    #[test]
    fn saved_callee_fill_judges_the_fat_envelope() {
        // A web whose envelope [5, 40) covers a holder [12, 20) is refused
        // even if it is dead across the holder: the tier deliberately uses
        // fat envelopes (hole-aware segments measured worse on zstd).
        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        occupied.insert(1, vec![(12, 20)]);
        let placed =
            fill_saved_callee_stretches(vec![(30, 35, 3, (5, 40))], &mut occupied, &[PhysReg(1)]);
        assert!(placed.is_empty());
        assert_eq!(
            occupied.get(&1).cloned().unwrap_or_default(),
            vec![(12, 20)]
        );
    }

    #[test]
    fn saved_callee_fill_breaks_ties_by_envelope_then_id_and_tries_each_register() {
        // Equal pressure: the shorter envelope goes first, then the lower id;
        // a candidate overlapping the first register's union falls through
        // to the next; a candidate born exactly where a holder dies takes
        // the register (half-open hand-off) and the union merges.
        let mut occupied: FxHashMap<u8, Vec<(u32, u32)>> = FxHashMap::default();
        occupied.insert(1, vec![(0, 10)]);
        let candidates = vec![
            (5, 9, 4, (18, 27)),
            (5, 3, 6, (12, 16)),
            (5, 3, 2, (14, 18)),
            (5, 9, 8, (12, 26)),
        ];
        let placed =
            fill_saved_callee_stretches(candidates, &mut occupied, &[PhysReg(1), PhysReg(2)]);
        assert_eq!(
            placed,
            vec![(2, PhysReg(1)), (6, PhysReg(2)), (4, PhysReg(1))]
        );
        assert_eq!(
            occupied.get(&1).cloned().unwrap_or_default(),
            vec![(0, 10), (14, 27)]
        );
        // v8 [12,26) overlaps both unions and there is no third register.
        assert_eq!(
            occupied.get(&2).cloned().unwrap_or_default(),
            vec![(12, 16)]
        );
    }

    #[test]
    fn summed_use_weight_saturates_instead_of_wrapping() {
        let use_count: FxHashMap<u32, u64> = [(1, u64::MAX), (2, 1)].into_iter().collect();
        assert_eq!(summed_use_weight(&[1, 2], &use_count), u64::MAX);
        assert_eq!(summed_use_weight(&[1], &use_count), u64::MAX);
        assert_eq!(summed_use_weight(&[9], &use_count), 0);
    }

    #[test]
    fn source_home_rejects_folded_read_in_dirty_window() {
        // A folded (hidden) SIB-index read of `src` after a dest redefinition
        // must veto exactly like an IR-visible use: the home no longer holds
        // the source value at that program point.
        let mut func = IrFunction::new("folded_dirty".to_string(), IrType::I64, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(1)),
                    rhs: Operand::Const(IrConst::I64(2)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(3)),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(5)),
                    rhs: Operand::Const(IrConst::I64(6)),
                    ty: IrType::I64,
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(5)),
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 8;
        let mut liveness = compute_live_intervals(&func);
        // Folded read of v5 at instruction 2 (block_start + 2), after the
        // v2 redefinition at instruction 1 dirtied the shared home.
        let base = liveness.block_starts[0];
        liveness.folded_read_points.insert(5, vec![base + 2]);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 2,
            backedge_src: 5,
            block_idx: 0,
            source_block_idx: 0,
            source_def_idx: 0,
            copy_idx: 3,
        };
        assert!(
            !source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "folded read of v5 in the dirty window must veto coalescing"
        );

        // The same shape without the folded read stays admissible: the v5
        // redefinition at instruction 2 refreshes the shared home, so the
        // copy reads the current source value.
        liveness.folded_read_points.remove(&5);
        assert!(
            source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "clean window without folded reads must survive"
        );

        // One `SourceHomeFacts` shared across queries (and path-compressed by
        // the hidden-read web lookups of the vetoed ones) answers exactly as
        // facts built per query: the facts depend only on `func`.
        let mut shared = SourceHomeFacts::new(&func);
        for folded in [true, false, true, false] {
            if folded {
                liveness.folded_read_points.insert(5, vec![base + 2]);
            } else {
                liveness.folded_read_points.remove(&5);
            }
            assert_eq!(
                source_home_survives_dest_redefs_in(&func, &liveness, &succs, &cand, &mut shared),
                source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
                "shared facts diverged (folded={folded})"
            );
        }
    }

    #[test]
    fn folded_consumer_rooted_in_dest_does_not_veto() {
        // Miniature zlib-ng adler32 `buf += 8`: the `GEP(buf, off)` loads are
        // folded-attributed to the increment via the latch copy, but the
        // address root is `dest` itself, so the shared home holds exactly
        // the value the access needs even in a dirty window.
        let mut func = IrFunction::new("folded_dest_root".to_string(), IrType::I64, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(9)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(11),
                    base: Value(2),
                    offset: Operand::Const(IrConst::I64(1)),
                    ty: IrType::Ptr,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::Load {
                    dest: Value(8),
                    ptr: Value(11),
                    ty: IrType::U8,
                    volatile: false,
                    seg_override: Default::default(),
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(2)),
                    ty: IrType::I64,
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(5)),
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 12;
        let mut liveness = compute_live_intervals(&func);
        let base = liveness.block_starts[0];
        liveness.folded_read_points.insert(5, vec![base + 5]);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 2,
            backedge_src: 5,
            block_idx: 0,
            source_block_idx: 0,
            source_def_idx: 3,
            copy_idx: 7,
        };
        assert!(
            source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "dest-rooted folded read must not veto (adler32 buf+=8 shape)"
        );

        // Control: the same folded read through a root that IS the source
        // still vetoes — the consumer reads the shared home itself while it
        // holds dest's newer value (true dirty-window hazard).
        let Instruction::GetElementPtr { base, .. } = &mut func.blocks[0].instructions[2] else {
            panic!("fixture shape changed");
        };
        *base = Value(5);
        assert!(
            !source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "src-rooted folded read in a dirty window must veto"
        );

        // Disjoint web: the same folded read through a two-peel chain
        // (GEP(v7) of folded address math v7 = v9+1 over the opaque root v9;
        // read-home distance 2) that provably occupies a different home (no
        // potential-sharing
        // edge to the candidate web) must NOT veto — the dest redef cannot
        // corrupt a home the consumer never reads (lz4 snapshot-chain
        // shape; single-peel reads stay vetoed, see the distance gate).
        let Instruction::GetElementPtr { base, .. } = &mut func.blocks[0].instructions[2] else {
            panic!("fixture shape changed");
        };
        *base = Value(7);
        assert!(
            source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "disjoint-web folded read in a dirty window must not veto"
        );

        // Distance gate: the same folded read one peel away (GEP over the
        // opaque root v9 directly; read-home distance 1) stays vetoed even
        // though v9 provably occupies a different home — single-peel
        // evidence is too weak to risk global recolor churn (expat len-2
        // regression guard).
        let Instruction::GetElementPtr { base, .. } = &mut func.blocks[0].instructions[2] else {
            panic!("fixture shape changed");
        };
        *base = Value(9);
        assert!(
            !source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "single-peel disjoint folded read must veto (distance gate)"
        );
    }

    #[test]
    fn hidden_read_through_deep_address_chain_does_not_veto() {
        // Miniature lz4 {v404,v224} shape: the consumer address (ptr v11)
        // folds through a deep address chain (v7 = v6+1, v6 = v9+1, v9
        // opaque) whose root provably occupies a different home (no
        // potential-sharing edge to the candidate web {2,5}), so the
        // dirty-window hidden read of the source must not veto. (Real lz4
        // paths are len-4 GEP/BinOp/Cast chains; Copies stop the peel, so
        // snapshot Copies are invisible here by construction.)
        let mut func = IrFunction::new("snapshot_chain".to_string(), IrType::I64, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(9)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(6)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(11),
                    base: Value(7),
                    offset: Operand::Const(IrConst::I64(1)),
                    ty: IrType::Ptr,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I64,
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(5)),
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::Load {
                    dest: Value(8),
                    ptr: Value(11),
                    ty: IrType::U8,
                    volatile: false,
                    seg_override: Default::default(),
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 12;
        let mut liveness = compute_live_intervals(&func);
        let base = liveness.block_starts[0];
        liveness.folded_read_points.insert(5, vec![base + 7]);
        let label_to_idx = analysis::build_label_map(&func);
        let (_preds, succs) = analysis::build_cfg(&func, &label_to_idx);
        let cand = PhiCoalesceCandidate {
            phi_dest: 2,
            backedge_src: 5,
            block_idx: 0,
            source_block_idx: 0,
            source_def_idx: 4,
            copy_idx: 5,
        };
        assert!(
            source_home_survives_dest_redefs(&func, &liveness, &succs, &cand),
            "hidden read through a snapshot chain into a disjoint web must not veto"
        );
    }

    #[test]
    fn homeless_merge_profitability_gate() {
        // Expat {v185,v20} shape: source covers 2 points disjoint from the
        // leader's global segments; the merged fat range would occupy ~40
        // new gap points to delete one cold Copy — refuse.
        let gap = vec![
            LiveInterval {
                value_id: 185,
                start: 0,
                end: 10,
            },
            LiveInterval {
                value_id: 185,
                start: 33,
                end: 87,
            },
            LiveInterval {
                value_id: 20,
                start: 127,
                end: 128,
            },
        ];
        assert!(
            !homeless_merge_profitable(&gap, 185, 20),
            "pure-gap merge (overlap 0, fill ~40) must be refused"
        );
        // lz4 {v404,v212} shape: full fat overlap — merge.
        let fat = vec![
            LiveInterval {
                value_id: 404,
                start: 0,
                end: 265,
            },
            LiveInterval {
                value_id: 212,
                start: 0,
                end: 265,
            },
        ];
        assert!(
            homeless_merge_profitable(&fat, 404, 212),
            "full-overlap merge (overlap 265, fill 0) must be accepted"
        );
        // Inside-disjoint: source sits in a hole of the leader's span —
        // no new span, shrinking fill — merge.
        let inside = vec![
            LiveInterval {
                value_id: 7,
                start: 0,
                end: 30,
            },
            LiveInterval {
                value_id: 7,
                start: 50,
                end: 100,
            },
            LiveInterval {
                value_id: 8,
                start: 40,
                end: 41,
            },
        ];
        assert!(
            homeless_merge_profitable(&inside, 7, 8),
            "inside-disjoint merge (no new span) must be accepted"
        );
    }
}

#[cfg(test)]
mod sse_destructive_tests {
    use super::*;
    use crate::ir::intrinsics::IntrinsicOp as O;
    use crate::ir::reexports::{BasicBlock, BlockId, Value};

    fn v(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    fn chain(dest: u32, op: O, a0: u32, a1: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op,
            dest_ptr: None,
            args: vec![v(a0), v(a1)],
        }
    }

    fn root(id: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(id),
            src: Operand::Const(IrConst::I32(0)),
        }
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn mkfunc(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("sse_chain".to_string(), IrType::Void, vec![], false);
        f.blocks = blocks;
        f.next_value_id = 100;
        f
    }

    fn run(func: &IrFunction, pool: &[PhysReg]) -> FxHashMap<u32, PhysReg> {
        let liveness = compute_live_intervals(func);
        let candidates = collect_sse128_chain_values(func);
        allocate_vector_registers_destructive(func, &candidates, pool, &|_| false, &liveness)
            .into_iter()
            .collect()
    }

    #[test]
    fn handoff_recycles_dying_first_operand_down_a_chain() {
        // v1 dies at the v3 def, v3 dies at the v4 def: both destinations
        // must take their first operand's register (the in-place form),
        // while the surviving second operand v2 keeps its own.
        let func = mkfunc(vec![block(
            0,
            vec![
                root(1),                        // @0
                root(2),                        // @1
                chain(3, O::VecXorI32x4, 1, 2), // @2
                chain(4, O::VecAddI32x4, 3, 2), // @3
            ],
            Terminator::Return(Some(v(4))),
        )]);
        let pool = vec![PhysReg(21), PhysReg(22)];
        let home = run(&func, &pool);
        for id in [1, 2, 3, 4] {
            assert!(home.contains_key(&id), "v{id} must be homed");
        }
        assert_eq!(home.get(&3), home.get(&1), "v3 must recycle v1's register");
        assert_eq!(home.get(&4), home.get(&3), "v4 must recycle v3's register");
        assert_ne!(
            home.get(&2),
            home.get(&3),
            "live second operand needs its own register"
        );
    }

    #[test]
    fn copy_defined_accumulator_survives_inner_short_value() {
        // P0: v2 is (re)defined by a phi-edge-style Copy at @1 but first
        // used at @5.  A mention model starts its span at the first USE and
        // lets the inner short-lived v3 [4,5] share the register — v3's
        // definition then clobbers the accumulator.  Segment coverage
        // starts at the definition, so the two must differ.
        let func = mkfunc(vec![block(
            0,
            vec![
                root(1), // @0
                Instruction::Copy {
                    dest: Value(2),
                    src: v(1),
                }, // @1
                root(8), // @2
                root(9), // @3
                chain(3, O::VecXorI32x4, 9, 8), // @4
                chain(5, O::VecAddI32x4, 2, 3), // @5
            ],
            Terminator::Return(Some(v(5))),
        )]);
        let pool = vec![PhysReg(21), PhysReg(22)];
        let home = run(&func, &pool);
        assert!(home.contains_key(&2), "accumulator must be homed");
        assert!(home.contains_key(&3), "inner value must be homed");
        assert_ne!(
            home.get(&2),
            home.get(&3),
            "accumulator and inner value overlap: different registers"
        );
        assert_eq!(
            home.get(&5),
            home.get(&2),
            "v5 must recycle its dying first operand v2's register"
        );
        assert_ne!(
            home.get(&5),
            home.get(&3),
            "in-place dest must not alias the live second operand"
        );
    }

    #[test]
    fn terminator_use_blocks_handoff() {
        // v1's last read is the block-0 CondBranch condition, AFTER the v3
        // def: no handoff edge may form (a mention model that skips
        // terminators would recycle v1's register into v3 and miscompile
        // the branch).
        let func = mkfunc(vec![
            block(
                0,
                vec![root(1), root(2), chain(3, O::VecXorI32x4, 1, 2)],
                Terminator::CondBranch {
                    cond: v(1),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
        ]);
        let pool = vec![PhysReg(21), PhysReg(22)];
        let home = run(&func, &pool);
        assert!(home.contains_key(&1), "v1 must be homed");
        assert!(home.contains_key(&3), "v3 must be homed");
        assert_ne!(
            home.get(&1),
            home.get(&3),
            "v1 is live in the terminator: v3 must not recycle its register"
        );
    }

    #[test]
    fn call_spanning_values_keep_no_xmm_home() {
        // v1 and v3 are live across the Memcpy call point: the caller-saved
        // pool must not home them.  v2 dies before, v5 is born after.
        let func = mkfunc(vec![block(
            0,
            vec![
                root(1),                        // @0
                root(2),                        // @1
                chain(3, O::VecXorI32x4, 1, 2), // @2
                root(6),                        // @3
                root(7),                        // @4
                Instruction::Memcpy {
                    dest: Value(6),
                    src: Value(7),
                    size: 16,
                }, // @5: call point
                chain(5, O::VecAddI32x4, 3, 1), // @6
            ],
            Terminator::Return(Some(v(5))),
        )]);
        let pool = vec![PhysReg(21), PhysReg(22)];
        let home = run(&func, &pool);
        assert!(!home.contains_key(&1), "v1 spans the call: no home");
        assert!(!home.contains_key(&3), "v3 spans the call: no home");
        assert!(home.contains_key(&2), "v2 dies before the call: homed");
        assert!(home.contains_key(&5), "v5 born after the call: homed");
    }

    #[test]
    fn allocation_is_deterministic() {
        let func = mkfunc(vec![block(
            0,
            vec![
                root(1),
                root(2),
                chain(3, O::VecXorI32x4, 1, 2),
                chain(4, O::VecAddI32x4, 3, 2),
            ],
            Terminator::Return(Some(v(4))),
        )]);
        let pool = vec![PhysReg(21), PhysReg(22), PhysReg(23)];
        let a = run(&func, &pool);
        let b = run(&func, &pool);
        assert_eq!(a, b, "two runs over the same IR must agree");
    }
}

#[cfg(test)]
mod fp_phi_move_tests {
    use super::*;

    fn phys(reg: u8) -> PhysReg {
        PhysReg(reg)
    }

    /// p20_sum_i64 shape: the blocker shares the dest reg and its FAT
    /// envelope overlaps the source, but no segment overlaps — the move
    /// must be allowed (the old fat-only test vetoed it, +8 movdqas).
    #[test]
    fn segment_disjoint_despite_fat_overlap_moves() {
        // Third value v21: fat (13,61), pieces avoid (51,54).
        let seg_cov: FxHashMap<u32, Vec<(u32, u32)>> = [
            (21u32, vec![(13, 22), (24, 24), (58, 61)]),
            (50u32, vec![(51, 54)]),
        ]
        .into_iter()
        .collect();
        let iv_map: FxHashMap<u32, (u32, u32)> =
            [(21u32, (13, 61)), (48u32, (10, 61)), (50u32, (51, 54))]
                .into_iter()
                .collect();
        // Dest v48 and blocker v21 both homed in 32; source v50 in 30.
        let assignments: FxHashMap<u32, PhysReg> =
            [(21u32, phys(32)), (48u32, phys(32)), (50u32, phys(30))]
                .into_iter()
                .collect();
        assert!(
            !fp_phi_move_conflicts(&seg_cov, &iv_map, &assignments, 48, 50, 32),
            "segment-disjoint blocker must not veto the phi move"
        );
    }

    #[test]
    fn true_segment_overlap_blocks() {
        let seg_cov: FxHashMap<u32, Vec<(u32, u32)>> =
            [(21u32, vec![(13, 22), (50, 52)]), (50u32, vec![(51, 54)])]
                .into_iter()
                .collect();
        let iv_map: FxHashMap<u32, (u32, u32)> =
            [(21u32, (13, 52)), (48u32, (10, 61)), (50u32, (51, 54))]
                .into_iter()
                .collect();
        let assignments: FxHashMap<u32, PhysReg> =
            [(21u32, phys(32)), (48u32, phys(32)), (50u32, phys(30))]
                .into_iter()
                .collect();
        assert!(
            fp_phi_move_conflicts(&seg_cov, &iv_map, &assignments, 48, 50, 32),
            "a third value live in the dest home during the source must veto"
        );
    }

    #[test]
    fn fat_fallback_for_unsegmented_values() {
        // Unsegmented blocker: fat envelope decides (conservative, sound).
        let seg_cov: FxHashMap<u32, Vec<(u32, u32)>> =
            [(50u32, vec![(51, 54)])].into_iter().collect();
        let iv_map: FxHashMap<u32, (u32, u32)> =
            [(21u32, (13, 61)), (48u32, (10, 61)), (50u32, (51, 54))]
                .into_iter()
                .collect();
        let assignments: FxHashMap<u32, PhysReg> =
            [(21u32, phys(32)), (48u32, phys(32)), (50u32, phys(30))]
                .into_iter()
                .collect();
        assert!(
            fp_phi_move_conflicts(&seg_cov, &iv_map, &assignments, 48, 50, 32),
            "unsegmented blocker with overlapping fat envelope must veto"
        );
        // Unsegmented source: fat envelope decides against segments.
        let seg_cov_src: FxHashMap<u32, Vec<(u32, u32)>> =
            [(21u32, vec![(50, 52)])].into_iter().collect();
        assert!(
            fp_phi_move_conflicts(&seg_cov_src, &iv_map, &assignments, 48, 50, 32),
            "unsegmented source with overlapping fat envelope must veto"
        );
    }

    #[test]
    fn ignores_other_regs_and_the_pair_itself() {
        // Overlapping values in other regs, plus the pair itself, never veto.
        let seg_cov: FxHashMap<u32, Vec<(u32, u32)>> = [
            (21u32, vec![(51, 54)]),
            (50u32, vec![(51, 54)]),
            (48u32, vec![(10, 61)]),
        ]
        .into_iter()
        .collect();
        let iv_map: FxHashMap<u32, (u32, u32)> =
            [(21u32, (51, 54)), (48u32, (10, 61)), (50u32, (51, 54))]
                .into_iter()
                .collect();
        let assignments: FxHashMap<u32, PhysReg> =
            [(21u32, phys(33)), (48u32, phys(32)), (50u32, phys(30))]
                .into_iter()
                .collect();
        assert!(
            !fp_phi_move_conflicts(&seg_cov, &iv_map, &assignments, 48, 50, 32),
            "other-reg holders and the pair itself must not veto"
        );
    }

    #[test]
    fn no_coverage_fails_closed() {
        let seg_cov: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
        let iv_map: FxHashMap<u32, (u32, u32)> = [(48u32, (10, 61))].into_iter().collect();
        let assignments: FxHashMap<u32, PhysReg> = [(48u32, phys(32))].into_iter().collect();
        assert!(
            fp_phi_move_conflicts(&seg_cov, &iv_map, &assignments, 48, 50, 32),
            "a source with no coverage info must fail closed"
        );
    }
}

#[cfg(test)]
mod scalar_lane_class_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Value};

    #[test]
    fn fp_lane_results_and_copies_are_scalar_but_integer_lanes_are_not() {
        use IntrinsicOp as O;
        let ops = [
            O::VecExtractLaneF64x2,
            O::VecExtractLaneF64x4,
            O::VecExtractLaneF32x4,
            O::VecExtractLaneF32x8,
            O::VecExtractLaneI64x2,
            O::VecExtractLaneI32x4,
        ];
        let mut f = IrFunction::new("lane_classes".into(), IrType::Void, vec![], false);
        let mut instructions: Vec<_> = ops
            .into_iter()
            .enumerate()
            .map(|(i, op)| Instruction::Intrinsic {
                dest: Some(Value(i as u32)),
                op,
                dest_ptr: None,
                args: vec![Operand::Value(Value(20)), Operand::Const(IrConst::I32(1))],
            })
            .collect();
        instructions.push(Instruction::Copy {
            dest: Value(8),
            src: Operand::Value(Value(0)),
        });
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        let scalars = collect_f64_values(&f, true);
        let conservative = collect_f64_values(&f, false);
        assert!(conservative.is_empty(), "lane homes must remain opt-in");
        let non_gpr = collect_non_gpr_values(&f, false);
        let types = crate::backend::common::compute_value_type_map(&f);
        for id in [0, 1, 2, 3, 8] {
            assert!(
                non_gpr.contains(&id),
                "v{id} must pass the GPR exclusion filter"
            );
            assert!(
                types.get(&id).is_some_and(is_scalar_fp),
                "v{id} needs scalar FP type"
            );
            assert!(scalars.contains(&id), "v{id}");
        }
        for id in [4, 5, 20] {
            assert!(!scalars.contains(&id), "v{id}");
        }
        for id in [4, 5] {
            assert!(
                !non_gpr.contains(&id),
                "integer lane v{id} must remain GPR-eligible"
            );
        }
    }

    #[test]
    fn decimal_copies_and_phis_are_never_gpr_homed() {
        // `c ? 1.5DD : 2.25DD` after phi elimination: two constant copies
        // into the merge value plus the D64 phi itself.  Every member is
        // FP-class on both word sizes (on i686 a GPR holds 4 of 8 bytes).
        let mut f = IrFunction::new("dec_phi".into(), IrType::Void, vec![], false);
        let bb = |label: u32, instructions: Vec<Instruction>| BasicBlock {
            label: BlockId(label),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        };
        f.blocks.push(bb(
            0,
            vec![
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::D64(0x31a0_0000_0000_000f)),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::D32(0x3280_0001)),
                },
                Instruction::Copy {
                    dest: Value(4),
                    src: Operand::Const(IrConst::I32(7)),
                },
            ],
        ));
        f.blocks.push(bb(
            1,
            vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::D64,
                incoming: vec![(Operand::Value(Value(1)), BlockId(0))],
            }],
        ));
        for is_32bit in [true, false] {
            let non_gpr = collect_non_gpr_values(&f, is_32bit);
            for id in [1, 2, 3] {
                assert!(
                    non_gpr.contains(&id),
                    "v{id} (decimal) is FP-class, 32-bit={is_32bit}"
                );
            }
            assert!(
                !non_gpr.contains(&4),
                "an int constant copy stays GPR-eligible"
            );
        }
    }

    #[test]
    fn phi_coalesce_admits_two_hop_short_circuit_latch_chain() {
        use crate::ir::reexports::IrCmpOp;
        // Rotated `while ((b & 0x80) != 0 && i < 5)` latch chain:
        // Block 1 (body) computes `i_next = i + 1` and branches to Block 2 (`i < 5`)
        // or Block 4 (exit); Block 2 branches to Block 3 (split latch copy `i = i_next`)
        // or Block 4 (exit). Block 3 has single-pred chain `1 -> 2 -> 3`.
        let mut f = IrFunction::new("varint_latch".into(), IrType::U32, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(10),
                src: Operand::Const(IrConst::I32(2)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(11),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(10)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::U32,
                },
                Instruction::Cmp {
                    dest: Value(12),
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(Value(11)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(12)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Cmp {
                dest: Value(13),
                op: IrCmpOp::Ult,
                lhs: Operand::Value(Value(11)),
                rhs: Operand::Const(IrConst::I32(5)),
                ty: IrType::U32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(13)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(10),
                src: Operand::Value(Value(11)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(11)))),
            source_spans: vec![],
        });
        f.next_value_id = 14;
        let liveness = compute_live_intervals(&f);
        let cands = detect_phi_coalesce_groups(&f, &liveness);
        assert!(
            cands
                .iter()
                .any(|c| c.phi_dest == 10 && c.backedge_src == 11),
            "expected (10, 11) phi coalesce across 2-hop latch chain, got {cands:?}"
        );
    }
}

/// Tests for the single definition of the phi-coalescer's chain shape.
///
/// `single_pred_chain` is pure graph analysis, so every case here is decided
/// by the predecessor sets alone — no liveness, no allocation, no codegen.
/// The liveness gate and the caller-saved-clobber window both consume it,
/// which is why the hop limit and the refusal cases are pinned here once.
#[cfg(test)]
mod phi_chain_shape_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId};

    /// Straight-line function `0 -> 1 -> ... -> n-1`.
    fn line(n: u32) -> IrFunction {
        let mut f = IrFunction::new("line".into(), IrType::U64, vec![], false);
        for i in 0..n {
            let terminator = if i + 1 == n {
                Terminator::Return(None)
            } else {
                Terminator::Branch(BlockId(i + 1))
            };
            f.blocks.push(BasicBlock {
                label: BlockId(i),
                instructions: vec![],
                terminator,
                source_spans: vec![],
            });
        }
        f
    }

    fn preds_of(f: &IrFunction) -> analysis::FlatAdj {
        analysis::build_cfg(f, &analysis::build_label_map(f)).0
    }

    #[test]
    fn direct_edge_is_an_empty_chain() {
        let f = line(3);
        let preds = preds_of(&f);
        assert_eq!(single_pred_chain(&preds, 0, 1), Some(Vec::new()));
        assert_eq!(single_pred_chain(&preds, 1, 2), Some(Vec::new()));
    }

    #[test]
    fn intermediate_blocks_are_ordered_source_to_copy() {
        let f = line(6);
        let preds = preds_of(&f);
        assert_eq!(single_pred_chain(&preds, 1, 3), Some(vec![2]));
        assert_eq!(single_pred_chain(&preds, 2, 5), Some(vec![3, 4]));
        assert_eq!(single_pred_chain(&preds, 0, 5), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn hop_limit_and_same_block_refuse() {
        let f = line(7);
        let preds = preds_of(&f);
        // Four intermediates are the contract; a fifth is refused rather
        // than silently truncated (a truncated window would let the clobber
        // check inspect fewer blocks than the liveness gate admitted).
        assert_eq!(single_pred_chain(&preds, 0, 5), Some(vec![1, 2, 3, 4]));
        assert_eq!(single_pred_chain(&preds, 0, 6), None);
        // Same block: not a chain at all.
        assert_eq!(single_pred_chain(&preds, 3, 3), None);
    }

    #[test]
    fn multi_predecessor_and_cycles_refuse() {
        // Diamond: 0 -> {1, 2} -> 3. Block 3 has two predecessors, so no
        // chain may reach it; block 1 is not the source of block 2.
        let mut f = IrFunction::new("diamond".into(), IrType::U64, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: vec![],
        });
        for i in 1..3u32 {
            f.blocks.push(BasicBlock {
                label: BlockId(i),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(3)),
                source_spans: vec![],
            });
        }
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        let preds = preds_of(&f);
        assert_eq!(preds.len(3), 2, "precondition: block 3 is a join");
        // No chain may END at a join: block 3 has two predecessors.
        assert_eq!(single_pred_chain(&preds, 1, 3), None);
        assert_eq!(single_pred_chain(&preds, 0, 3), None);
        // The diamond arms are still directly reachable from the entry, so
        // those ARE empty chains -- reachability, not topology, decides.
        assert_eq!(single_pred_chain(&preds, 0, 1), Some(Vec::new()));
        assert_eq!(single_pred_chain(&preds, 0, 2), Some(Vec::new()));
        assert_eq!(single_pred_chain(&preds, 1, 2), None);

        // Back edge: 0 -> 1 -> 2 -> 1. The walk stops at block 1 because the
        // back edge gives it two predecessors (the `mids.contains` guard is
        // the belt-and-braces for a shape where it would not).
        let mut g = IrFunction::new("cycle".into(), IrType::U64, vec![], false);
        g.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        g.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: vec![],
        });
        g.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        let preds = preds_of(&g);
        assert_eq!(single_pred_chain(&preds, 0, 2), None);
    }
}

#[cfg(test)]
mod save_set_compact_tests {
    //! RA-SAVE-SET-COMPACT: a callee-saved register's whole holder set folds
    //! into another saved register only when no holder pair meets.
    use super::{FxHashMap, PhysReg, compact_callee_saved_homes};

    fn run(
        homes: &[(u32, u8, Option<(u32, u32)>)],
    ) -> (FxHashMap<u32, PhysReg>, Vec<(PhysReg, PhysReg)>) {
        let mut assignments = FxHashMap::default();
        let mut iv_map = FxHashMap::default();
        for &(v, r, iv) in homes {
            assignments.insert(v, PhysReg(r));
            if let Some(iv) = iv {
                iv_map.insert(v, iv);
            }
        }
        let pool = [PhysReg(1), PhysReg(2), PhysReg(3)];
        let moves = compact_callee_saved_homes(&mut assignments, &iv_map, &pool);
        (assignments, moves)
    }

    #[test]
    fn folds_a_register_whose_holders_all_died() {
        // find_bit shape: r14's v53 [34,35] fits beside rbx's v28 [15,16].
        let (a, moves) = run(&[(28, 1, Some((15, 16))), (53, 2, Some((34, 35)))]);
        assert_eq!(moves.len(), 1, "{moves:?}");
        assert_eq!(a[&28], a[&53], "{a:?}");
    }

    #[test]
    fn overlap_and_touching_holders_block_the_fold() {
        let (_, moves) = run(&[(1, 1, Some((0, 10))), (2, 2, Some((5, 15)))]);
        assert!(moves.is_empty(), "overlap folded: {moves:?}");
        // Inclusive: a death point shared with a new co-tenant is refused.
        let (_, moves) = run(&[(1, 1, Some((0, 5))), (2, 2, Some((5, 9)))]);
        assert!(moves.is_empty(), "touching folded: {moves:?}");
    }

    #[test]
    fn the_whole_holder_set_moves_or_nothing_does() {
        // r2's v3 overlaps r1's v1, so r2 cannot fold even though v2 fits;
        // r1 cannot fold into r2 either. Nobody may move alone.
        let (a, moves) = run(&[
            (1, 1, Some((1, 3))),
            (2, 2, Some((10, 12))),
            (3, 2, Some((0, 2))),
        ]);
        assert!(moves.is_empty(), "{moves:?}");
        assert_eq!(a[&2], PhysReg(2));
        assert_eq!(a[&1], PhysReg(1));
    }

    #[test]
    fn a_holder_without_an_interval_pins_its_register() {
        let (a, moves) = run(&[(1, 1, None), (2, 2, Some((10, 12)))]);
        assert!(moves.is_empty(), "{moves:?}");
        assert_eq!(a[&2], PhysReg(2));
    }
}

/// Hidden call SEQUENCES (dynamic TLS on AArch64/RISC-V) and the i686 pool
/// predicate: what each may and may not change in the allocation.
#[cfg(test)]
mod call_sequence_policy_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst, Value};

    const SPANNING: std::ops::Range<u32> = 20..26;

    /// `a`, `b` feed six products that are all live across `&tv` and summed
    /// after it; `a`/`b` themselves die before the TLS access. The shape that
    /// spilled both parameters when the TLS sequence drove Phase 1.
    fn tls_pressure_func() -> IrFunction {
        let v = |n: u32| Operand::Value(Value(n));
        let mut insts = vec![
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty: IrType::I64,
            },
            Instruction::ParamRef {
                dest: Value(2),
                param_idx: 1,
                ty: IrType::I64,
            },
        ];
        for (k, d) in SPANNING.enumerate() {
            insts.push(Instruction::BinOp {
                dest: Value(d - 10),
                op: IrBinOp::Mul,
                lhs: v(1),
                rhs: Operand::Const(IrConst::I64(k as i64 + 3)),
                ty: IrType::I64,
            });
            insts.push(Instruction::BinOp {
                dest: Value(d),
                op: IrBinOp::Add,
                lhs: v(d - 10),
                rhs: v(2),
                ty: IrType::I64,
            });
        }
        insts.push(Instruction::GlobalAddr {
            dest: Value(30),
            name: "tv".to_string(),
        });
        insts.push(Instruction::Load {
            dest: Value(31),
            ptr: Value(30),
            ty: IrType::I64,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        });
        let mut acc = 31;
        for (k, d) in SPANNING.enumerate() {
            let dest = 40 + k as u32;
            insts.push(Instruction::BinOp {
                dest: Value(dest),
                op: IrBinOp::Add,
                lhs: v(acc),
                rhs: v(d),
                ty: IrType::I64,
            });
            acc = dest;
        }
        let mut f = IrFunction::new("tls_pressure".to_string(), IrType::I64, vec![], false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(v(acc))),
            source_spans: Vec::new(),
        }];
        f.next_value_id = 50;
        f
    }

    fn config(caller_saved_regs: Vec<PhysReg>) -> RegAllocConfig {
        RegAllocConfig {
            ipa_regs: Vec::new(),
            available_regs: vec![PhysReg(1), PhysReg(2), PhysReg(3)],
            accumulator_policy: AccumulatorPolicy {
                operand_order: AccumulatorOperandOrder::LhsFirst,
                return_consumes_accumulator: false,
            },
            caller_saved_regs,
            call_arg_regs: vec![],
            indirect_target_regs: vec![],
            allow_inline_asm_regalloc: false,
            leaf_caller_saved_homes: false,
            xmm_regs: Vec::new(),
            never_materialized: FxHashSet::default(),
            folded_index_uses: FxHashMap::default(),
            fold_base_regs: Vec::new(),
            fold_readmit: FxHashSet::default(),
            reg_hints: FxHashMap::default(),
            ra_config: Arc::new(RaConfig::default()),
        }
    }

    /// Allocate with `tv` installed as a call sequence (`seq`) or not.
    fn allocate(f: &IrFunction, cfg: &RegAllocConfig, seq: bool) -> FxHashMap<u32, PhysReg> {
        let names: FxHashSet<String> = if seq {
            std::iter::once("tv".to_string()).collect()
        } else {
            FxHashSet::default()
        };
        crate::common::tls::set_call_sequence_symbols(names);
        let result = allocate_registers(f, cfg);
        crate::common::tls::set_call_sequence_symbols(FxHashSet::default());
        result.assignments
    }

    #[test]
    fn sequences_are_recorded_as_a_subset_of_call_points() {
        let f = tls_pressure_func();
        crate::common::tls::set_call_sequence_symbols(std::iter::once("tv".into()).collect());
        let live = crate::backend::liveness::compute_live_intervals(&f);
        crate::common::tls::set_call_sequence_symbols(FxHashSet::default());
        assert_eq!(live.sequence_call_points.len(), 1);
        assert_eq!(live.call_points, live.sequence_call_points);
        let plain = crate::backend::liveness::compute_live_intervals(&f);
        assert!(plain.call_points.is_empty() && plain.sequence_call_points.is_empty());
    }

    /// With only callee-saved registers on offer, a call sequence has nothing
    /// to protect, so it must not change a single home. It used to promote the
    /// six spanning products into Phase 1 ahead of the parameters.
    #[test]
    fn sequences_do_not_reorder_callee_saved_priority() {
        let f = tls_pressure_func();
        let cfg = config(vec![]);
        let plain = allocate(&f, &cfg, false);
        let with_seq = allocate(&f, &cfg, true);
        assert!(
            plain.contains_key(&1) && plain.contains_key(&2),
            "fixture premise: the parameters keep homes ({plain:?})"
        );
        let sorted = |m: &FxHashMap<u32, PhysReg>| {
            let mut v: Vec<(u32, u8)> = m.iter().map(|(&k, r)| (k, r.0)).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(sorted(&with_seq), sorted(&plain));
    }

    /// Eligibility is unchanged: no caller-saved home spans a call sequence.
    #[test]
    fn no_caller_saved_home_spans_a_sequence() {
        let f = tls_pressure_func();
        let pool = vec![PhysReg(13), PhysReg(14)];
        let cfg = config(pool.clone());
        let plain = allocate(&f, &cfg, false);
        assert!(
            SPANNING
                .clone()
                .any(|d| plain.get(&d).is_some_and(|r| pool.contains(r))),
            "fixture premise: without the sequence a spanning value takes the pool ({plain:?})"
        );
        let with_seq = allocate(&f, &cfg, true);
        for d in SPANNING {
            assert!(
                !with_seq.get(&d).is_some_and(|r| pool.contains(r)),
                "v{d} spans the TLS sequence in a caller-saved home: {with_seq:?}"
            );
        }
    }

    /// PhysReg 4/5 are %ecx/%edx only on i686; AArch64's x4/x5 share the
    /// numbers and must take the generic Phase 2.
    #[test]
    fn i686_phases_key_on_the_target_not_the_register_numbers() {
        let saved = crate::common::types::target_elf_machine();
        let arm_pool = [4u8, 5, 6, 7, 8, 13, 14].map(PhysReg).to_vec();
        for (machine, pool, expected) in [
            (3u16, vec![PhysReg(5), PhysReg(4)], true),
            (183, arm_pool.clone(), false),
            (62, arm_pool, false),
            (3, vec![], false),
        ] {
            crate::common::types::set_target_elf_machine(machine);
            assert_eq!(
                i686_scratch_pool(&config(pool)),
                expected,
                "e_machine {machine}"
            );
        }
        crate::common::types::set_target_elf_machine(saved);
    }
}

#[cfg(test)]
mod fold_defer_tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, IrParam, Value};

    /// `p = param; a = p+8; b = p+16; return *a + *b` — both GEPs are
    /// constant-offset fold addresses of base `p` (value 0).
    fn two_field_loads() -> IrFunction {
        let ld = |dest, ptr| Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        let gep = |dest, off| Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::Ptr,
        };
        let mut f = IrFunction::new(
            "fold_defer".to_string(),
            IrType::I64,
            vec![IrParam {
                ty: IrType::Ptr,
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
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                gep(1, 8),
                gep(2, 16),
                ld(3, 1),
                ld(4, 2),
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(4)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
            source_spans: Vec::new(),
        }];
        f.next_value_id = 6;
        f
    }

    fn config(fold_base_regs: Vec<PhysReg>) -> RegAllocConfig {
        RegAllocConfig {
            ipa_regs: Vec::new(),
            available_regs: vec![PhysReg(1), PhysReg(2), PhysReg(3)],
            accumulator_policy: AccumulatorPolicy {
                operand_order: AccumulatorOperandOrder::LhsFirst,
                return_consumes_accumulator: false,
            },
            caller_saved_regs: Vec::new(),
            call_arg_regs: vec![],
            indirect_target_regs: vec![],
            allow_inline_asm_regalloc: false,
            leaf_caller_saved_homes: false,
            xmm_regs: Vec::new(),
            never_materialized: FxHashSet::default(),
            folded_index_uses: FxHashMap::default(),
            fold_base_regs,
            fold_readmit: FxHashSet::default(),
            reg_hints: FxHashMap::default(),
            ra_config: Arc::new(RaConfig::default()),
        }
    }

    fn sorted(mut v: Vec<u32>) -> Vec<u32> {
        v.sort_unstable();
        v
    }

    #[test]
    fn deferral_skips_readmitted_addresses_and_ineligible_bases() {
        let f = two_field_loads();
        let all: FxHashSet<u32> = (0..6).collect();

        let mut eligible = all.clone();
        let t = defer_const_fold_addresses(&f, &mut eligible, &FxHashSet::default());
        assert_eq!(sorted(t[&0].clone()), vec![1, 2]);
        assert!(!eligible.contains(&1) && !eligible.contains(&2));
        assert!(eligible.contains(&0));

        let mut eligible = all.clone();
        let readmit: FxHashSet<u32> = std::iter::once(1).collect();
        let t = defer_const_fold_addresses(&f, &mut eligible, &readmit);
        assert_eq!(t[&0], vec![2]);
        assert!(eligible.contains(&1) && !eligible.contains(&2));

        // Every address re-admitted: no entry at all (not an empty list).
        let mut eligible = all.clone();
        let readmit: FxHashSet<u32> = [1, 2].into_iter().collect();
        assert!(defer_const_fold_addresses(&f, &mut eligible, &readmit).is_empty());
        assert_eq!(eligible, all);

        // A base that is no candidate cannot carry its addresses.
        let mut eligible: FxHashSet<u32> = all.iter().copied().filter(|&v| v != 0).collect();
        let before = eligible.clone();
        assert!(defer_const_fold_addresses(&f, &mut eligible, &FxHashSet::default()).is_empty());
        assert_eq!(eligible, before);
    }

    #[test]
    fn homed_base_keeps_its_addresses_deferred_in_one_round() {
        let f = two_field_loads();
        let mut cfg = config(vec![PhysReg(1), PhysReg(2), PhysReg(3)]);
        let r = allocate_registers_with_fold_rounds(&f, &mut cfg);
        let base = r.assignments.get(&0).copied();
        assert!(
            base.is_some_and(|p| cfg.fold_base_regs.contains(&p)),
            "{base:?}"
        );
        assert!(cfg.fold_readmit.is_empty());
        assert_eq!(sorted(r.fold_deferred[&0].clone()), vec![1, 2]);
        assert!(!r.assignments.contains_key(&1) && !r.assignments.contains_key(&2));
    }

    #[test]
    fn failed_base_readmits_its_addresses_and_reaches_a_fixpoint() {
        let f = two_field_loads();
        // No allocatable register is foldable: every deferral must fail and
        // be undone, after which nothing is deferred and the loop stops.
        let mut cfg = config(vec![PhysReg(200)]);
        let r = allocate_registers_with_fold_rounds(&f, &mut cfg);
        assert_eq!(cfg.fold_readmit, [1, 2].into_iter().collect());
        assert!(r.fold_deferred.is_empty());
        assert!(r.eligible.contains(&1) && r.eligible.contains(&2));
    }

    #[test]
    fn empty_fold_pool_disables_deferral() {
        let f = two_field_loads();
        let mut cfg = config(Vec::new());
        let r = allocate_registers_with_fold_rounds(&f, &mut cfg);
        assert!(r.fold_deferred.is_empty() && cfg.fold_readmit.is_empty());
    }
}

#[cfg(test)]
mod wide_hazard_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, Value};

    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }

    fn cast(dest: u32, src: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: v(src),
            from_ty,
            to_ty,
        }
    }

    #[test]
    fn integer_truncation_out_of_i128_has_a_gpr_result() {
        use IrType::*;
        for (from, to, is_32bit, gpr) in [
            (I128, U32, false, true),
            (U128, I64, false, true),
            (I128, I8, true, true),
            // On i686 an I64 is a register pair, not one GPR.
            (I128, I64, true, false),
            // Into or across wide types, or out to a float: not a GPR result.
            (I64, I128, false, false),
            (I128, U128, false, false),
            (I128, F64, false, false),
            (I32, I64, false, true),
        ] {
            assert_eq!(
                cast_result_is_gpr(from, to, is_32bit),
                gpr,
                "{from:?} -> {to:?} 32bit={is_32bit}"
            );
        }
    }

    /// Points: 0 ParamRef I128, 1 ParamRef I64, 2 lshr I128, 3 trunc I128->U32,
    /// 4 trunc U32->I16, 5 add I64, 6 zext I64->I128, 7 I128 -> F64 (libcall),
    /// 8 cmp I128 (opens the fusion window), 9 add I64, 10 terminator.
    #[test]
    fn integer_casts_across_the_wide_boundary_are_not_scratch_hazards() {
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::I128,
                },
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 1,
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::LShr,
                    lhs: v(1),
                    rhs: Operand::Const(IrConst::I64(64)),
                    ty: IrType::I128,
                },
                cast(3, 2, IrType::I128, IrType::U32),
                cast(4, 3, IrType::U32, IrType::I16),
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: v(0),
                    rhs: v(0),
                    ty: IrType::I64,
                },
                cast(6, 0, IrType::I64, IrType::I128),
                cast(7, 1, IrType::I128, IrType::F64),
                Instruction::Cmp {
                    dest: Value(8),
                    op: IrCmpOp::Ult,
                    lhs: v(6),
                    rhs: v(1),
                    ty: IrType::I128,
                },
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: v(5),
                    rhs: v(0),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(v(9))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 10;
        assert_eq!(
            collect_x64_wide_hazard_points(&f),
            vec![0, 2, 7, 8, 9, 10],
            "integer casts 3 and 6 stage through rax:rdx only; the F64 cast \
             is a libcall; the I128 cmp's fusion window runs to the terminator"
        );
    }
}

#[cfg(test)]
mod ipa_fill_tests {
    use super::*;

    const R8: PhysReg = PhysReg(12);
    const R9: PhysReg = PhysReg(13);
    const R11: PhysReg = PhysReg(10);
    const R10: PhysReg = PhysReg(11);
    const RBX: PhysReg = PhysReg(1);
    const R12: PhysReg = PhysReg(2);

    /// Values >= 100 are displaceable Phase-2 tenants; below are call-spanning.
    fn tenant_ok(v: u32, _: PhysReg) -> bool {
        v >= 100
    }

    struct Case {
        assignments: FxHashMap<u32, PhysReg>,
        iv: FxHashMap<u32, (u32, u32)>,
        cost: FxHashMap<u32, u64>,
    }

    impl Case {
        fn new(vals: &[(u32, (u32, u32), u64, Option<PhysReg>)]) -> Self {
            let mut c = Case {
                assignments: FxHashMap::default(),
                iv: FxHashMap::default(),
                cost: FxHashMap::default(),
            };
            for &(v, iv, cost, home) in vals {
                c.iv.insert(v, iv);
                c.cost.insert(v, cost);
                if let Some(r) = home {
                    c.assignments.insert(v, r);
                }
            }
            c
        }

        #[allow(clippy::too_many_arguments)]
        fn run(
            &mut self,
            candidates: &[u32],
            ipa: &[PhysReg],
            callee: &[PhysReg],
            reloc: &[PhysReg],
            reloc_ok: &dyn Fn(u32, PhysReg) -> bool,
            blocked: &dyn Fn(u32) -> bool,
            demand: &[(u32, u32)],
        ) -> Vec<PhysReg> {
            let cost = self.cost.clone();
            let iv = self.iv.clone();
            ipa_fill_preserved_regs(
                &mut self.assignments,
                candidates,
                &IpaFill {
                    ipa_regs: ipa,
                    callee_pool: callee,
                    reloc_pool: reloc,
                    iv_map: &iv,
                    cost: &|v| cost.get(&v).copied().unwrap_or(0),
                    reloc_ok,
                    rename_blocked: blocked,
                    scratch_demand: demand,
                    trace: false,
                },
            )
        }
    }

    const POOL: &[PhysReg] = &[R11, R10, R8, R9];

    #[test]
    fn homes_the_costliest_candidate_first() {
        // v1 and v2 overlap; one IPA register: the costlier v2 wins it.
        let mut c = Case::new(&[(1, (0, 10), 5, None), (2, (5, 15), 9, None)]);
        let used = c.run(&[1, 2], &[R8], &[], POOL, &tenant_ok, &|_| false, &[]);
        assert_eq!(used, vec![R8]);
        assert_eq!(c.assignments.get(&2), Some(&R8));
        assert_eq!(c.assignments.get(&1), None, "a home is never shared");
    }

    #[test]
    fn zero_cost_and_intervalless_candidates_stay_slotted() {
        let mut c = Case::new(&[(1, (0, 10), 0, None)]);
        assert!(
            c.run(&[1, 7], &[R8], &[], POOL, &tenant_ok, &|_| false, &[])
                .is_empty()
        );
        assert!(c.assignments.is_empty());
    }

    #[test]
    fn shared_endpoint_conflicts_and_displaces_the_tenant() {
        // Tenant v100 [10,20] in r8; candidate v1 [20,30] touches it at 20:
        // inclusive overlap, so v100 must move (to the first free, allowed
        // relocation register) before v1 takes r8.
        let mut c = Case::new(&[(100, (10, 20), 1, Some(R8)), (1, (20, 30), 4, None)]);
        let used = c.run(&[1], &[R8], &[], POOL, &tenant_ok, &|_| false, &[]);
        assert_eq!(used, vec![R8]);
        assert_eq!(c.assignments.get(&1), Some(&R8));
        assert_eq!(c.assignments.get(&100), Some(&R11));
    }

    #[test]
    fn displacement_respects_the_tenant_register_rules() {
        // v100 may not live in r11 (e.g. a fold base); r10 is the next one.
        let ok = |v: u32, r: PhysReg| v >= 100 && r != R11;
        let mut c = Case::new(&[(100, (10, 20), 1, Some(R8)), (1, (0, 30), 4, None)]);
        c.run(&[1], &[R8], &[], POOL, &ok, &|_| false, &[]);
        assert_eq!(c.assignments.get(&100), Some(&R10));
        // A tenant that may not move at all pins its register.
        let mut c = Case::new(&[(100, (10, 20), 1, Some(R8)), (1, (0, 30), 4, None)]);
        let used = c.run(&[1], &[R8], &[], POOL, &|_, _| false, &|_| false, &[]);
        assert!(used.is_empty());
        assert_eq!(c.assignments.get(&100), Some(&R8));
        assert_eq!(c.assignments.get(&1), None);
    }

    #[test]
    fn displaced_group_moves_together_and_never_into_a_busy_register() {
        // Tenants v100 [0,5] and v101 [5,9] share r8 (touching, as Phase 2's
        // half-open spans allow); r11 is busy over [3,4], r10 is free.
        let mut c = Case::new(&[
            (100, (0, 5), 1, Some(R8)),
            (101, (5, 9), 1, Some(R8)),
            (102, (3, 4), 1, Some(R11)),
            (1, (0, 9), 4, None),
        ]);
        c.run(&[1], &[R8], &[], POOL, &tenant_ok, &|_| false, &[]);
        assert_eq!(c.assignments.get(&100), Some(&R10));
        assert_eq!(c.assignments.get(&101), Some(&R10));
        assert_eq!(c.assignments.get(&102), Some(&R11));
        assert_eq!(c.assignments.get(&1), Some(&R8));
    }

    #[test]
    fn renames_a_whole_callee_saved_register_unless_blocked() {
        // rbx holds a two-value web; r12 holds a parameter (blocked).
        let mut c = Case::new(&[
            (10, (0, 4), 3, Some(RBX)),
            (11, (6, 9), 3, Some(RBX)),
            (12, (0, 9), 3, Some(R12)),
        ]);
        let used = c.run(
            &[],
            &[R8, R9],
            &[RBX, R12],
            POOL,
            &tenant_ok,
            &|v| v == 12,
            &[],
        );
        assert_eq!(used, vec![R8]);
        assert_eq!(c.assignments.get(&10), Some(&R8));
        assert_eq!(c.assignments.get(&11), Some(&R8));
        assert_eq!(c.assignments.get(&12), Some(&R12));
    }

    #[test]
    fn keeps_one_caller_saved_register_free_for_emitter_scratch() {
        // Pool r11/r8 only; r11 busy over the candidate's whole range: taking
        // r8 would leave no free caller-saved register there.
        let pool = &[R11, R8];
        let mut c = Case::new(&[(100, (0, 30), 1, Some(R11)), (1, (0, 30), 9, None)]);
        assert!(
            c.run(&[1], &[R8], &[], pool, &tenant_ok, &|_| false, &[])
                .is_empty()
        );
        // One more free register in the pool: admitted.
        let mut c = Case::new(&[(100, (0, 30), 1, Some(R11)), (1, (0, 30), 9, None)]);
        assert_eq!(
            c.run(
                &[1],
                &[R8],
                &[],
                &[R11, R10, R8],
                &tenant_ok,
                &|_| false,
                &[]
            ),
            vec![R8]
        );
        // ... unless a homeless window value already claims it over [12,14].
        let mut c = Case::new(&[(100, (0, 30), 1, Some(R11)), (1, (0, 30), 9, None)]);
        let pool = &[R11, R10, R8];
        assert!(
            c.run(&[1], &[R8], &[], pool, &tenant_ok, &|_| false, &[(12, 14)])
                .is_empty()
        );
    }

    #[test]
    fn an_unknown_tenant_interval_freezes_its_register() {
        let mut c = Case::new(&[(1, (0, 9), 9, None)]);
        c.assignments.insert(100, R8); // no interval: occupancy unknown
        assert!(
            c.run(&[1], &[R8], &[], POOL, &tenant_ok, &|_| false, &[])
                .is_empty()
        );
        assert_eq!(c.assignments.get(&100), Some(&R8));
    }

    #[test]
    fn untouched_without_ipa_registers() {
        let mut c = Case::new(&[(1, (0, 9), 9, None), (10, (0, 4), 3, Some(RBX))]);
        assert!(
            c.run(&[1], &[], &[RBX], POOL, &tenant_ok, &|_| false, &[])
                .is_empty()
        );
        assert_eq!(c.assignments.len(), 1);
        assert_eq!(c.assignments.get(&10), Some(&RBX));
    }
}
