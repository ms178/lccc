//! Exact intra-function register liveness for the x86-64 peephole passes.
//!
//! Every pass that deletes a definition needs one question answered: *is this
//! register read on any path from here?* The passes used to answer it with two
//! syntactic approximations — a block-local write-before-read scan and a
//! whole-function "no other mention" test. Both are sound but blind: they miss
//! the common case where a register is written in a loop body and its only
//! other mention is a read in the PROLOGUE, which no back edge reaches.
//!
//! This module computes the real answer: basic blocks, a successor graph, and
//! a backward dataflow fixpoint over the 16 general-purpose register families.
//!
//! # Conservative by construction
//!
//! * A function is analysed only when it is delimited by
//!   `.cfi_startproc`/`.cfi_endproc` and **every** control transfer inside it
//!   is resolvable: a `jmp`/`jCC` to a label defined in the same function, a
//!   `call`, or a `ret`. An indirect jump, a jump table, a tail call to a
//!   symbol, or a label that is never targeted by a resolvable branch but sits
//!   in the middle of the function is fine — what is *not* fine is a transfer
//!   whose destination is unknown, which marks the whole function unanalysable
//!   ([`FileLiveness::live_after`] then answers `None` and callers fall back to
//!   their syntactic proofs).
//! * Instructions are classified by mnemonic. An unrecognised mnemonic is
//!   assumed to READ every register it mentions and to write nothing, so its
//!   operands stay live.
//! * Inline assembly reads and writes everything.
//! * At `ret`, the return-value registers (`%rax`, `%rdx`) and the callee-saved
//!   registers (`%rbx`, `%rbp`, `%rsp`, `%r12`..`%r15`) are live: the ABI
//!   requires their values to be intact.
//! * A `call` reads the SysV argument registers, `%rax` (variadic vector count)
//!   and `%r10` (static chain), and clobbers the caller-saved set.
//! * A sibling call's `jmp`, published by the codegen with a
//!   `# LCCC_TAIL_CALL` marker block, is a call plus a return: it has no
//!   successor, reads what the call reads (same marker protocol) and the
//!   callee-saved registers. An unmarked jump to a symbol stays unresolvable.

use super::super::types::*;
use super::helpers::{get_dest_reg, is_read_modify_write, second_token, src_mentions_family};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// All 16 GP families.
const ALL: u16 = 0xFFFF;
/// rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8..r15 = 8..15.
const RAX: u16 = 1 << 0;
const RCX: u16 = 1 << 1;
const RDX: u16 = 1 << 2;
const RBX: u16 = 1 << 3;
const RSP: u16 = 1 << 4;
const RBP: u16 = 1 << 5;
/// Values an ABI-visible transfer reads without naming them: the six SysV
/// argument registers and `%rax` (variadic vector count). `%r10` (the
/// static chain) is NOT here: chain calls carry `# LCCC_CHAIN_CALL`
/// (the codegen arms it from the SetStaticChain emission that stages the
/// chain immediately before every nested call — LCCC's IR inserts that
/// staging at every nested call site, so no other shape can pass a chain
/// invisibly). Modeling every call as reading %r10 pinned each %r10
/// scratch value across every call — the RA's most common scratch pick
/// after rax/rcx/rdx — and blocked the LEA→memory window fold.
///
/// The six-argument-register read is itself now refined per call site by
/// the `# LCCC_CALL_ARGS <n>` marker (see the `LineKind::Call` arm): a
/// call reads exactly the FIRST n GP argument registers its classification
/// consumed. This constant survives as the fail-closed default for absent
/// markers (hand-written fragments, raw `call` emissions).
#[expect(dead_code)]
const CALL_READS: u16 = RAX | RCX | RDX | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9);
/// Registers a call may destroy.
pub(super) const CALLER_SAVED: u16 =
    RAX | RCX | RDX | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11);
/// Live at `ret`: integer return value plus everything the callee must restore.
const RET_LIVE: u16 = RAX | RDX | RBX | RBP | RSP | (1 << 12) | (1 << 13) | (1 << 14) | (1 << 15);
/// Same, for a function that demonstrably returns a single value in `%rax`
/// (its epilogue writes the accumulator and never touches `%rdx`). Keeping
/// `%rdx` artificially live there hides every dead `movq %rax, %rdx` the
/// return-value materialisation leaves behind.
const RET_LIVE_RAX_ONLY: u16 = RET_LIVE & !RDX;
/// Live at a sibling call's `jmp`: the callee-saved registers (restored by
/// the epilogue, owed to our caller, preserved by the tail callee).
const TAIL_CALL_LIVE: u16 = RET_LIVE & !(RAX | RDX);

/// Whether the `jmp` on line `n` is a codegen-published sibling call: its
/// marker block opens with `# LCCC_TAIL_CALL` (`x86::codegen::returns`),
/// followed by the suppressed call's own markers, which
/// [`call_site_reads`] parses from the same position.
pub(super) fn tail_call_marked(store: &LineStore, infos: &[LineInfo], n: usize) -> bool {
    (n + 1..(n + 1 + CALL_MARKER_WINDOW).min(infos.len()))
        .find(|&k| !infos[k].is_nop())
        .is_some_and(|k| infos[k].trimmed(store.get(k)) == "# LCCC_TAIL_CALL")
}

/// How many lines after a `call` its marker protocol may occupy: every
/// marker [`call_markers`] consults sits on lines `n+1..=n+CALL_MARKER_WINDOW`
/// of the call on line `n`, so an edit further down cannot change that
/// call's read set. The codegen publishes at most five markers per call
/// (`VA_CALL`, `CALL_ARGS`, `CHAIN_CALL`, `CALL_FP`, `IPA_KEEP`) directly
/// after the call text; one line of slack.
const CALL_MARKER_WINDOW: usize = 6;

/// Entries per generation of the function-liveness memo (`analyse_function`).
const FN_CACHE_CAP: usize = 4096;
/// Cached lines per generation.  The memo's cost is per LINE, not per entry: a
/// cached function holds `fn_len` records of `size_of::<CachedLine>()` (10)
/// bytes.  An entry-count cap alone let 4096 entries of 20k-line functions
/// retain ~800 MB per generation, so the line total is what rotates the memo.
/// `1 << 20` lines is ~10 MiB per generation and ~20 MiB with the previous one.
const FN_CACHE_LINE_BUDGET: usize = 1 << 20;
/// Shortest function worth memoising.  A miss pays a hash, an allocation and a
/// map insert, which a sub-16-line solve does not earn back.
const FN_CACHE_MIN_LINES: usize = 16;
const _: () = assert!(FN_CACHE_MIN_LINES <= FN_CACHE_LINE_BUDGET);

/// Whether a memo generation must rotate before admitting `extra` more lines.
/// An empty generation never rotates, so one oversized function is still
/// memoised (its own size bounds that single entry).
fn fn_cache_gen_full(entries: usize, lines: usize, extra: usize) -> bool {
    entries != 0 && (entries >= FN_CACHE_CAP || lines.saturating_add(extra) > FN_CACHE_LINE_BUDGET)
}

/// The marker protocol of the `call` on line `n`, parsed from the contiguous
/// block of comment/directive lines that directly follows the call text.
///
/// The block ends at the first real instruction or label, which keeps a *later
/// call's* markers out — but comments do not end it, so a marker block that was
/// published without a call text above it (a stranded sibling-call block, see
/// `x86::codegen::calls::emit_call_marker`) IS reachable from the call above
/// it. Every counted field therefore latches its FIRST legible occurrence: the
/// codegen publishes each marker exactly once per call text, directly after it,
/// so the first one in the block is this call's own and any later one belongs
/// to a different call and must never overwrite it.
///
/// One parser for every consumer. The variadic and static-chain markers used
/// to be searched only on lines `n+1..=n+2`, which a variadic static-chain
/// call (`[call][VA_CALL][CALL_ARGS][CHAIN_CALL]`) violates: the chain read
/// of %r10 was invisible and its staging write deletable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallMarkers {
    /// `# LCCC_VA_CALL`: the callee reads %al (SSE register count).
    pub(crate) variadic: bool,
    /// `# LCCC_CHAIN_CALL`: the callee reads the static chain in %r10.
    pub(crate) chain: bool,
    /// `# LCCC_CALL_ARGS <n>`: leading SysV GP argument registers read
    /// (6, the fail-closed default, when absent or illegible).
    pub(crate) gp_args: usize,
    /// `# LCCC_IPA_KEEP <mask>`: caller-saved GP families the caller keeps
    /// live ACROSS this call because the callee provably never writes them
    /// (interprocedural register allocation, `x86::codegen::ipa_ra`). The
    /// oracles model the call as reading them (and, through the ABI clobber
    /// set, writing them): the strongest conservative reading of "preserved"
    /// that every pass already handles, since it is exactly how they treat
    /// argument registers.
    pub(crate) ipa_keep: u16,
}

pub(crate) fn call_markers(store: &LineStore, infos: &[LineInfo], n: usize) -> CallMarkers {
    let mut m = CallMarkers {
        variadic: false,
        chain: false,
        gp_args: 6,
        ipa_keep: 0,
    };
    // `variadic` and `chain` are monotone: a second block can only add reads,
    // which is the conservative direction. `gp_args` and `ipa_keep` are values,
    // so each latches its first legible occurrence (see the fn docs).
    let mut gp_args_seen = false;
    let mut ipa_keep_seen = false;
    for k in n + 1..(n + 1 + CALL_MARKER_WINDOW).min(infos.len()) {
        if infos[k].is_nop() {
            continue;
        }
        let mk = infos[k].trimmed(store.get(k));
        // A real instruction ends the marker block, and so does a label: it
        // opens another block, whose first transfer may publish its own
        // markers right there (a sibling call's `jmp` repeats the suppressed
        // call's `# LCCC_CALL_ARGS` ahead of the jump). Reading past `.LBB2:`
        // once attributed `jmp abort@PLT`'s zero-argument marker to the
        // `jmp exit@PLT` above it, and the `xorl %edi, %edi` staging exit's
        // argument was deleted as dead (gcc.c-torture 981130-1, 980526-2).
        if infos[k].kind == LineKind::Label || !(mk.starts_with('#') || mk.starts_with('.')) {
            break;
        }
        if mk.starts_with("# LCCC_VA_CALL") {
            m.variadic = true;
        } else if mk.starts_with("# LCCC_CHAIN_CALL") {
            m.chain = true;
        } else if let Some(rest) = mk.strip_prefix("# LCCC_CALL_ARGS ") {
            // First legible marker wins, and an illegible one still counts as
            // this call's (fail-closed to all six argument registers, exactly
            // as an absent marker does) so a later block cannot sneak in.
            // Last-wins once let a stranded sibling-call block downgrade a
            // 1-argument call to 0 arguments: `%rdi` became provably dead and
            // `eliminate_dead_pure_writes` deleted the argument staging of the
            // call above it (Linux 6.18.63 `addrconf_verify_work`, guest panic
            // at t=0.87 s).
            debug_assert!(
                !gp_args_seen,
                "two `# LCCC_CALL_ARGS` markers in one call's window: the \
                 protocol publishes exactly one per call text"
            );
            if !gp_args_seen {
                gp_args_seen = true;
                m.gp_args = rest.trim_ascii().parse::<usize>().map_or(6, |v| v.min(6));
            }
        } else if let Some(rest) = mk.strip_prefix("# LCCC_IPA_KEEP ") {
            // Fail-closed parse: an illegible mask keeps every caller-saved
            // family live (read) at the call. First legible marker wins, for
            // the same reason as `# LCCC_CALL_ARGS`.
            debug_assert!(
                !ipa_keep_seen,
                "two `# LCCC_IPA_KEEP` markers in one call's window: the \
                 protocol publishes exactly one per call text"
            );
            if !ipa_keep_seen {
                ipa_keep_seen = true;
                m.ipa_keep = rest
                    .trim_ascii()
                    .strip_prefix("0x")
                    .and_then(|h| u16::from_str_radix(h, 16).ok())
                    .map_or(CALLER_SAVED, |v| v & CALLER_SAVED);
            }
        }
    }
    m
}

/// Registers the `call` on line `n` reads: the leading SysV GP argument
/// registers its `# LCCC_CALL_ARGS <n>` marker publishes (all six when the
/// marker is absent), `%rax` at `# LCCC_VA_CALL` sites, `%r10` at static-chain
/// and external-retpoline sites, plus every register its own text mentions.
/// The single authority for call-site reads: [`FileLiveness`] and the
/// windowed dead-register scans (`call_window_verdict`) both consult it, so
/// the two liveness oracles cannot disagree about a call.
pub(super) fn call_site_reads(store: &LineStore, infos: &[LineInfo], n: usize) -> u16 {
    let t = infos[n].trimmed(store.get(n));
    let mentioned = infos[n].reg_refs;
    // A VARIADIC callee reads %rax (the live SSE register count
    // in %al — SysV AMD64 3.5.7); a prototyped non-variadic
    // callee reads nothing from the accumulator. The codegen
    // marks every variadic site with `# LCCC_VA_CALL`
    // immediately after the call text (same authority contract
    // as the `# LCCC_RET_*` prologue markers): absent marker ⇒
    // the accumulator is unread here. Dropping the false read
    // unlocks copy/extension folding up to the call in every
    // non-variadic caller (the gzip_crc32 harness kept
    // `movzbl+movq+movb` chains live solely on it).
    let markers = call_markers(store, infos, n);
    let variadic = markers.variadic;
    // A static-chain callee reads the chain staged in %r10 by
    // the SetStaticChain emission directly before this call;
    // `# LCCC_CHAIN_CALL` marks exactly those sites. The
    // staging write's own liveness DEPENDS on this read: without
    // it, dead-write elimination would retire the staging and
    // silently drop the chain.
    let chain = markers.chain;
    // External-retpoline indirect call: the target register is read by
    // the THUNK symbol, not by this instruction's %-tokens. The register
    // named by the symbol suffix (`__x86_indirect_thunk_rax` reads %rax)
    // is decoded into this line's `reg_refs` — and therefore `mentioned` —
    // by the classifier (see `indirect_thunk_refs`); the call-site
    // lowering's own form always stages the target in %r10
    // (emit_call_spill_fptr_impl). The re-OR below is fail-closed for
    // LineInfos that bypassed the classifier (hand-built test fixtures):
    // `indirect_thunk_refs` answers every bit the name implies — the
    // decoded family, or the historical %r10 for an unrecognised suffix —
    // and OR-ing it again is idempotent when `mentioned` is already exact.
    // The inline-thunk and `call *%r10` forms mention %r10 in their own
    // text (`mentioned` covers them).
    let retpoline_thunk_bits = if t.starts_with("call __x86_indirect_thunk_") {
        super::super::types::indirect_thunk_refs(&t)
    } else {
        0
    };
    // `# LCCC_CALL_ARGS <n>`: the codegen's authoritative count
    // of leading SysV GP argument registers THIS call reads
    // (armed by the register-argument phase from the
    // `CallArgClass` classification, published right after the
    // call text — the same authority contract as the VA marker).
    // The conservative model reads all six at every call, which
    // pins any argument-register value across every call and
    // blocks folds whenever the RA homes a scratch in %rdx/
    // %rcx/... (measured: the hash-chain chase lost its
    // load→compare fold when the two-block unroller's
    // profitability gate shifted the register allocation).
    // Absent/illegible marker (hand-written fragments, raw
    // `call` emissions like the i128 helpers) keeps all six:
    // fail-closed.
    let gp_args = markers.gp_args;
    // The first n GP argument registers, in SysV order.
    let arg_reads: u16 = match gp_args {
        0 => 0,
        1 => 1 << 7,                                                // rdi
        2 => (1 << 7) | (1 << 6),                                   // +rsi
        3 => (1 << 7) | (1 << 6) | RDX,                             // +rdx
        4 => (1 << 7) | (1 << 6) | RDX | RCX,                       // +rcx
        5 => (1 << 7) | (1 << 6) | RDX | RCX | (1 << 8),            // +r8
        _ => (1 << 7) | (1 << 6) | RDX | RCX | (1 << 8) | (1 << 9), // +r9
    };
    let mut reads = if variadic {
        arg_reads | RAX | mentioned
    } else {
        arg_reads | mentioned
    };
    if chain {
        reads |= 1 << 10;
    }
    reads | retpoline_thunk_bits | markers.ipa_keep
}

/// Verdict of a forward windowed scan for GP family `reg` that reaches the
/// `call` on line `n`:
/// * `Some(false)`: live — the call reads it (argument, variadic count,
///   static chain, indirect target), or the target is a LOCAL label whose
///   code the linear window cannot follow;
/// * `Some(true)`: dead — an external callee clobbers the caller-saved
///   family without reading it;
/// * `None`: transparent — a callee-saved family survives the call
///   unchanged, so the scan must continue past it. Concluding "dead" here
///   deleted the only write of a value read after the call
///   (`movslq %eax, %rbx; movq %rbx, %rdi; call malloc` folded to
///   `movslq %eax, %rdi` while `memset` later read `%rbx`).
pub(super) fn call_window_verdict(
    store: &LineStore,
    infos: &[LineInfo],
    n: usize,
    reg: RegId,
) -> Option<bool> {
    let mask = 1u16 << reg;
    if call_site_reads(store, infos, n) & mask != 0 {
        return Some(false);
    }
    let t = infos[n].trimmed(store.get(n));
    if second_token(t).is_some_and(|target| target.starts_with('.')) {
        return Some(false);
    }
    if CALLER_SAVED & mask != 0 {
        Some(true)
    } else {
        None
    }
}

/// Per-line liveness for a whole assembly file.
/// Exact, lazy liveness upkeep for a forward-scanning pass that only
/// DELETES straight-line definitions (the dead-write passes).
///
/// Re-solving the whole function after every deletion made such a pass
/// O(deletions x function length).  By
/// [`FileLiveness::deletion_changes_only_reads`], a deletion leaves every
/// family's solution unchanged except the families the deleted line read,
/// so the stale solution still answers every other family exactly; the
/// function is re-solved only when a later query in it asks about one of
/// those families.  A function the scan has left is never queried again.
pub(super) struct DeadWriteRefresh {
    /// A line of the function with pending deletions, and the families
    /// whose solution there may have shrunk since it was last solved.
    stale: Option<(usize, u16)>,
}

impl DeadWriteRefresh {
    pub(super) fn new() -> Self {
        DeadWriteRefresh { stale: None }
    }

    /// Make `lv` exact for a query about `fam` after line `idx`.
    pub(super) fn before_query(
        &mut self,
        lv: &mut FileLiveness,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        fam: RegId,
    ) {
        let Some((k, families)) = self.stale else {
            return;
        };
        if !lv.same_function(k, idx) {
            // Forward scan: the stale function is behind us for good.
            self.stale = None;
        } else if fam <= REG_GP_MAX && families & (1u16 << fam) != 0 {
            lv.refresh_at(store, infos, k);
            self.stale = None;
        }
    }

    /// Delete line `idx` (`mark_nop`) and keep `lv` exact.
    pub(super) fn delete(
        &mut self,
        lv: &mut FileLiveness,
        store: &LineStore,
        infos: &mut [LineInfo],
        idx: usize,
    ) {
        let reads = lv.deletion_changes_only_reads(idx);
        mark_nop(&mut infos[idx]);
        self.stale = match (reads, self.stale) {
            (Some(r), Some((k, families))) if lv.same_function(k, idx) => Some((k, families | r)),
            (Some(r), _) => Some((idx, r)),
            (None, _) => {
                // Re-solves the whole function, pending deletions included.
                lv.refresh_at(store, infos, idx);
                None
            }
        };
    }
}

/// Saved span of one function inside a [`FileLiveness`]; see
/// [`FileLiveness::snapshot_function`].
pub(super) struct FnSpanSnapshot {
    start: usize,
    end: usize,
    live_out: Vec<u16>,
    live_in: Vec<u16>,
    known: Vec<bool>,
    structural: Vec<bool>,
    effect: Vec<(u16, u16)>,
    textual_ret: bool,
}

#[derive(Clone)]
pub(super) struct FileLiveness {
    /// Registers live immediately AFTER each line, when its function could be
    /// analysed.
    live_out: Vec<u16>,
    /// Registers live immediately BEFORE each line (same validity as
    /// `live_out`).  Kept so `refresh_span` can splice a local recomputation
    /// into the function's solution and prove it changed nothing outside.
    live_in: Vec<u16>,
    /// Whether the line belongs to an analysable function.
    known: Vec<bool>,
    /// Whether the line shaped the CFG or the function-wide context at the
    /// last analysis (see `LineInfo::structural`).  `refresh_span` must know the
    /// PRE-edit shape of the lines it re-derives, and the edit has already
    /// replaced their text.
    structural: Vec<bool>,
    /// `(reads, writes)` of each known line at the last analysis: the
    /// PRE-edit transfer functions `refresh_span`'s exactness test needs.
    effect: Vec<(u16, u16)>,
    /// `(start, end)` of the function each line belongs to (`start` is the
    /// `.cfi_startproc` line, `end` the `.cfi_endproc` line or the file
    /// length), `None` outside every function.
    func_of: Vec<Option<(u32, u32)>>,
    /// Functions whose `ret` read set was inferred from their text (no
    /// legible `# LCCC_RET_RDX` marker): any edit can change it.
    textual_ret: FxHashSet<usize>,
}

/// Reusable per-thread scratch for one function solve.  Every buffer here
/// used to be a fresh heap allocation per `analyse_function` call — and the
/// solver runs once per function per refresh, so DHAT attributed 1.46M
/// blocks / 907 MB of churn to it on the profiling TU.  Pooled, each vector
/// re-allocates only at a new high-water mark (`clear` + `resize` keep
/// capacity), the same amortisation `succ_flat` already documented.
/// Thread-local like `FN_CACHE` below, and for the same reason: the solver
/// is not re-entrant, so one scratch per thread is exactly enough.
struct SolveScratch {
    effects: Vec<Option<Effect>>,
    succ_range: Vec<(usize, usize)>,
    lines: Vec<usize>,
    succ_flat: Vec<usize>,
    live_in: Vec<u16>,
    live_out: Vec<u16>,
}

thread_local! {
    static SOLVE_SCRATCH: std::cell::RefCell<SolveScratch> =
        const {
            std::cell::RefCell::new(SolveScratch {
                effects: Vec::new(),
                succ_range: Vec::new(),
                lines: Vec::new(),
                succ_flat: Vec::new(),
                live_in: Vec::new(),
                live_out: Vec::new(),
            })
        };
}

// Counts memo hits, so the tests can prove a hit happened rather than
// inferring it from matching answers. Compiled out of release builds.
#[cfg(test)]
thread_local! {
    pub(super) static LIVENESS_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MEMO_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static EFFECT_MEMO_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Sweeps the last `solve_upper32_zero` needed (convergence-rate test).
    static UPPER_SWEEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `CCC_VERIFY_LIVENESS_SPAN` (see `FileLiveness::assert_matches_full_analysis`).
fn verify_span_refresh() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_VERIFY_LIVENESS_SPAN").is_some())
}

/// One `#APP` .. `#NO_APP` region whose text was read: `first..=last` are its
/// lines, `reads` the general registers it may read, `targets` the lines of
/// the function's labels it names (an asm goto target, an exception fixup, a
/// jump-table entry the kernel patches into a jump).
struct AsmRegion {
    first: usize,
    last: usize,
    reads: u16,
    targets: Vec<usize>,
}

/// The inline-asm regions of `start..end` whose effect on the general
/// registers can be told from their text.
///
/// Inline asm used to read every register.  That is always sound, and it
/// switched liveness off around the statement: whatever was computed on a
/// path to it stayed "read", so no temporary before a static key, a per-CPU
/// update or a `lock` prefix could be folded away -- which is most kernel
/// functions.  GCC's contract is narrower and is what the statement's author
/// wrote against: an asm reads its input operands.  Their registers are
/// printed into the template, so the text names every register the statement
/// reads, provided each instruction in it reads only what it names or what
/// its mnemonic implies (`types::classify_implicit_operands`).
///
/// A region is read only when every statement in it is one of:
///
/// * an instruction of [`asm_mnemonic_is_modelled`] -- the plain integer
///   instructions, and the ones whose implicit registers are tabulated;
/// * a jump to a label of this function, or to a numeric label the region
///   defines itself;
/// * a label, a section switch that is balanced within the region, alignment,
///   data in a section that is not executable, or one of the multi-byte NOPs
///   spelled as `.byte`s.
///
/// Anything else keeps the old answer for the whole region: a `call`, `ret`,
/// `syscall` or trap (a handler with a register contract of its own), an
/// indirect or foreign jump, an assembler macro, data in the instruction
/// stream or in an executable section (`.altinstr_replacement` holds code
/// that is patched in here), a directive not listed (`.intel_syntax` would
/// hide every register), an unknown mnemonic.
///
/// Writes are never claimed: the region kills nothing.  With that, the order
/// and the control flow of the statements inside cannot matter for what is
/// live in front of the region, and instructions inside pushed sections are
/// simply counted as if they ran here.
fn asm_regions(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    end: usize,
    resolve: &dyn Fn(&str) -> Option<usize>,
) -> Vec<AsmRegion> {
    let mut regions = Vec::new();
    let mut open: Option<usize> = None;
    for n in start..end {
        if infos[n].is_nop() {
            continue;
        }
        let text = infos[n].trimmed(store.get(n));
        if infos[n].kind != LineKind::InlineAsm {
            match text {
                "#APP" => open = Some(n + 1),
                "#NO_APP" => {
                    if let Some(first) = open.take().filter(|&first| first < n) {
                        if let Some((reads, targets)) =
                            read_asm_region(store, infos, first, n, resolve)
                        {
                            regions.push(AsmRegion {
                                first,
                                last: n - 1,
                                reads,
                                targets,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    regions
}

/// The registers lines `first..past` of one inline-asm region may read and
/// the function labels they name; `None` when a statement is not understood.
fn read_asm_region(
    store: &LineStore,
    infos: &[LineInfo],
    first: usize,
    past: usize,
    resolve: &dyn Fn(&str) -> Option<usize>,
) -> Option<(u16, Vec<usize>)> {
    let mut reads = 0u16;
    let mut targets: Vec<usize> = Vec::new();
    // Sections pushed inside the region: whether their bytes may execute.
    let mut sections: Vec<bool> = Vec::new();
    let mut numeric_defs: Vec<&str> = Vec::new();
    let mut numeric_refs: Vec<&str> = Vec::new();
    for n in first..past {
        if infos[n].kind != LineKind::InlineAsm {
            // A marker or a deleted line inside a region: not this shape.
            return None;
        }
        let raw = infos[n].trimmed(store.get(n));
        let line = &raw
            [..crate::backend::peephole_common::line_len_without_trailing_comment(raw.as_bytes())];
        // `;` separates statements, except inside a string.
        if line.contains('"') && line.contains(';') {
            return None;
        }
        for statement in line.split(';') {
            let mut s = statement.trim();
            // Leading labels: `1:`, `671:`, `.Lretry:`.
            while let Some((head, rest)) = s.split_once(':') {
                let symbol = !head.is_empty()
                    && head
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'));
                if !symbol {
                    break;
                }
                if head.bytes().all(|b| b.is_ascii_digit()) {
                    numeric_defs.push(head);
                }
                s = rest.trim_start();
            }
            if s.is_empty() {
                continue;
            }
            // Function labels the statement names, wherever it names them.
            for token in
                s.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')))
            {
                if token.starts_with('.') {
                    if let Some(target) = resolve(token) {
                        if !targets.contains(&target) {
                            targets.push(target);
                        }
                    }
                }
            }
            let (word, args) = s
                .split_once(|c: char| c.is_ascii_whitespace())
                .unwrap_or((s, ""));
            if word.starts_with('.') {
                let executes = sections.last().copied().unwrap_or(true);
                match word {
                    ".pushsection" | ".section" => sections.push(asm_section_may_execute(args)),
                    ".popsection" | ".previous" => {
                        sections.pop()?;
                    }
                    ".balign" | ".p2align" | ".align" | ".nops" => {}
                    ".byte" => {
                        if executes && !asm_bytes_are_a_nop(args) {
                            return None;
                        }
                    }
                    ".word" | ".short" | ".value" | ".2byte" | ".4byte" | ".8byte" | ".long"
                    | ".int" | ".quad" | ".ascii" | ".asciz" | ".string" | ".skip" | ".space"
                    | ".zero" | ".fill" | ".org" | ".uleb128" | ".sleb128" => {
                        if executes {
                            return None;
                        }
                    }
                    _ => return None,
                }
                continue;
            }
            let (_, mnemonic) = mnemonic_after_prefixes(s);
            let Some(mnemonic) = mnemonic else {
                // Prefix words alone (`rep; nop`, `lock; addl ...`): a
                // repeat prefix counts in %rcx whatever follows it.
                if s.split_ascii_whitespace().any(|w| w.starts_with("rep")) {
                    reads |= RCX;
                }
                continue;
            };
            if asm_mnemonic_is_jump(mnemonic) {
                let target = s
                    .rsplit(|c: char| c.is_ascii_whitespace())
                    .next()
                    .unwrap_or("");
                match target.strip_suffix(['b', 'f']) {
                    Some(digits)
                        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
                    {
                        numeric_refs.push(digits);
                    }
                    _ => {
                        // A label of this function, whatever its spelling;
                        // anything else leaves the function.
                        let line = resolve(target)?;
                        if !targets.contains(&line) {
                            targets.push(line);
                        }
                    }
                }
                continue;
            }
            if is_control_transfer_mnemonic(mnemonic) || !asm_mnemonic_is_modelled(mnemonic) {
                return None;
            }
            reads |= scan_register_refs(s.as_bytes());
        }
    }
    let balanced = sections.is_empty();
    let internal = numeric_refs.iter().all(|r| numeric_defs.contains(r));
    (balanced && internal).then_some((reads, targets))
}

/// Whether the bytes of the section a `.pushsection`/`.section` names may be
/// executed: its flags say so, or it gives none (fail closed).
fn asm_section_may_execute(args: &str) -> bool {
    let mut parts = args.splitn(2, ',');
    let _name = parts.next();
    let Some(rest) = parts.next() else {
        return true;
    };
    let rest = rest.trim_start();
    let Some(flags) = rest
        .strip_prefix('"')
        .and_then(|r| r.split_once('"'))
        .map(|(flags, _)| flags)
    else {
        return true;
    };
    flags.contains('x')
}

/// Whether a `.byte` list is one of the NOPs the kernel spells as bytes
/// (`BYTES_NOP1`..`BYTES_NOP8`; a static key's default is the 5-byte one).
fn asm_bytes_are_a_nop(args: &str) -> bool {
    const NOPS: [&[u8]; 8] = [
        &[0x90],
        &[0x66, 0x90],
        &[0x0f, 0x1f, 0x00],
        &[0x0f, 0x1f, 0x40, 0x00],
        &[0x0f, 0x1f, 0x44, 0x00, 0x00],
        &[0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00],
        &[0x0f, 0x1f, 0x80, 0x00, 0x00, 0x00, 0x00],
        &[0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
    ];
    let mut bytes = [0u8; 8];
    let mut len = 0usize;
    for item in args.split(',') {
        let item = item.trim();
        let value = match item.strip_prefix("0x").or_else(|| item.strip_prefix("0X")) {
            Some(hex) => u8::from_str_radix(hex, 16),
            None => item.parse::<u8>(),
        };
        let (Ok(value), Some(slot)) = (value, bytes.get_mut(len)) else {
            return false;
        };
        *slot = value;
        len += 1;
    }
    NOPS.contains(&&bytes[..len])
}

/// The condition a `j<cc>`, `set<cc>` or `cmov<cc>` tests.
fn asm_condition(code: &str) -> bool {
    const CONDITIONS: [&str; 30] = [
        "o", "no", "b", "c", "nae", "ae", "nb", "nc", "e", "z", "ne", "nz", "be", "na", "a", "nbe",
        "s", "ns", "p", "pe", "np", "po", "l", "nge", "ge", "nl", "le", "ng", "g", "nle",
    ];
    CONDITIONS.contains(&code)
}

/// `jmp` and the flag-conditional jumps: they read no general register.
/// (`jrcxz` and `loop` do, and are refused with the other transfers.)
fn asm_mnemonic_is_jump(mnemonic: &str) -> bool {
    mnemonic == "jmp" || mnemonic.strip_prefix('j').is_some_and(asm_condition)
}

/// Instructions an inline-asm region may contain and still be read: each
/// reads the registers its operands name, plus the implicit ones
/// `types::classify_implicit_operands` tabulates (the last group below).
/// The list is closed on purpose -- an instruction that is not here makes
/// its region read everything, as before.  Left out although common: the
/// double shifts (GAS reads `shld %a, %b` as a shift by `%cl`), `in`/`out`,
/// the x87/SSE state instructions, and everything that transfers control.
fn asm_mnemonic_is_modelled(mnemonic: &str) -> bool {
    // Integer instructions that take a width suffix and name every register
    // they read (`shll %cl, %eax` names its count).
    const SUFFIXED: [&str; 38] = [
        "mov", "lea", "add", "adc", "sub", "sbb", "and", "or", "xor", "cmp", "test", "inc", "dec",
        "neg", "not", "bt", "bts", "btr", "btc", "bsf", "bsr", "tzcnt", "lzcnt", "popcnt", "xchg",
        "xadd", "shl", "shr", "sar", "sal", "rol", "ror", "rcl", "rcr", "bswap", "nop", "push",
        "pop",
    ];
    // Exact spellings: extensions, flag and barrier instructions.
    const EXACT: [&str; 43] = [
        "movabs",
        "movabsq",
        "movzbw",
        "movzbl",
        "movzbq",
        "movzwl",
        "movzwq",
        "movsbw",
        "movsbl",
        "movsbq",
        "movswl",
        "movswq",
        "movslq",
        "pause",
        "lfence",
        "mfence",
        "sfence",
        "clac",
        "stac",
        "cli",
        "sti",
        "cld",
        "std",
        "clc",
        "stc",
        "cmc",
        "endbr64",
        "endbr32",
        "prefetcht0",
        "prefetcht1",
        "prefetcht2",
        "prefetchnta",
        "prefetchw",
        "clflush",
        "clflushopt",
        "clwb",
        "movnti",
        "movntil",
        "movntiq",
        "pushf",
        "pushfq",
        "popf",
        "popfq",
    ];
    // Implicit registers, tabulated.
    const IMPLICIT: [&str; 49] = [
        "cmpxchg", "cmpxchgb", "cmpxchgw", "cmpxchgl", "cmpxchgq", "cbtw", "cwtl", "cltq", "cwtd",
        "cltd", "cqto", "div", "divb", "divw", "divl", "divq", "idiv", "idivb", "idivw", "idivl",
        "idivq", "mul", "mulb", "mulw", "mull", "mulq", "imul", "imulb", "imulw", "imull", "imulq",
        "cpuid", "rdtsc", "rdtscp", "rdmsr", "wrmsr", "rdpmc", "xgetbv", "xsetbv", "movsb",
        "movsw", "movsl", "movsq", "stosb", "stosw", "stosl", "stosq", "lodsb", "lodsq",
    ];
    let suffixed = SUFFIXED.contains(&mnemonic)
        || mnemonic
            .strip_suffix(['b', 'w', 'l', 'q'])
            .is_some_and(|stem| SUFFIXED.contains(&stem));
    let conditional = |prefix: &str, width: bool| {
        mnemonic.strip_prefix(prefix).is_some_and(|code| {
            asm_condition(code)
                || (width
                    && code
                        .strip_suffix(['w', 'l', 'q'])
                        .is_some_and(asm_condition))
        })
    };
    suffixed
        || EXACT.contains(&mnemonic)
        || IMPLICIT.contains(&mnemonic)
        || conditional("set", false)
        || conditional("cmov", true)
}

/// Read/write sets of one instruction.
#[derive(Clone, Copy)]
struct Effect {
    reads: u16,
    writes: u16,
}

/// Mnemonics whose destination operand is written without being read.
fn is_pure_write_mnemonic(t: &str) -> bool {
    // SINGLE SOURCE OF TRUTH: the dest-only contract lives in
    // `helpers::is_read_modify_write`.  This module used to keep its own
    // copy of the list; the two drifted (rorx was missing here), every
    // `rorxl` became a phantom read of its destination, the K[i]/W[i]
    // families stayed live around the sha256 loop, and the load→add
    // fusions died (~9% runtime on Raptor Lake-class cores).  Delegating
    // makes drift structurally impossible.
    //
    // Call-site contract that makes the delegation exact:
    // * string instructions (`rep movsb`, ...) short-circuit BEFORE this
    //   predicate via `is_string_instruction`, so the `movs` len<=5
    //   exclusion the old list carried is a dead path here;
    // * the caller consults the result only for GP destinations
    //   (`dest <= REG_GP_MAX`), so scalar-SSE look-alikes whose dest is
    //   an xmm register (`movsd %xmm0, ...`) never reach it;
    // * the caller applies its own operand-text guards (source mentions
    //   the dest, partial-width dest), subsuming the LEA self-address
    //   exception the helper carries.
    // `cmov` stays read-modify-write (kept when the condition is false);
    // `mulx` stays conservative (implicit %rdx read, two destinations).
    !is_read_modify_write(t)
}

/// `true` for the x86 string instructions (`movs*`, `stos*`, `lods*`,
/// `scas*`, `cmps*`, with or without a `rep`/`repe`/`repne` prefix).  They
/// name no register in their text yet read and write `%rcx` (count),
/// `%rsi`/`%rdi` (pointers) and `%rax` (`stos`/`lods`/`scas` data).  Without
/// this the backward dataflow saw `movq $4096, %rcx; rep movsb; ret` as a
/// dead write to `%rcx` and deleted the count set-up (observed miscompile of
/// every block copy lowered to `rep movsb`, including the `-mno-sse` kernel
/// path).  The suffix test rejects the look-alikes `movsbl %al, %eax`
/// (sign-extending move) and `movsd %xmm0, ...` (scalar double move).
pub(crate) fn is_string_instruction(t: &str) -> bool {
    if !matches!(t.as_bytes().first(), Some(b'r' | b'm' | b's' | b'l' | b'c')) {
        return false;
    }
    let mut m = t;
    if m.as_bytes()[0] == b'r' {
        for p in ["repne ", "repnz ", "repe ", "repz ", "rep "] {
            if let Some(rest) = m.strip_prefix(p) {
                m = rest.trim_ascii_start();
                break;
            }
        }
    }
    let m = m.split_whitespace().next().unwrap_or("");
    if m.len() != 5 && m.len() != 4 {
        return false;
    }
    let (stem, suffix) = m.split_at(4);
    let string_stem = matches!(stem, "movs" | "stos" | "lods" | "scas" | "cmps");
    let size_suffix = suffix.is_empty() || matches!(suffix, "b" | "w" | "l" | "q");
    // A string op never takes operands in AT&T output of this backend; an
    // explicit operand list (`movsd %xmm0, ...`) marks a non-string form.
    string_stem && size_suffix && !t.contains('%')
}

fn mnemonic_is_known(t: &str) -> bool {
    let b = t.as_bytes();
    if b.len() >= 3 {
        let fast = match b[0] {
            b'm' => b.starts_with(b"mov") || b.starts_with(b"mul"),
            b'l' => b.starts_with(b"lea") || b.starts_with(b"lzcnt") || b.starts_with(b"leave"),
            b'a' => b.starts_with(b"add") || b.starts_with(b"and") || b.starts_with(b"adc"),
            b's' => {
                b.starts_with(b"sub")
                    || b.starts_with(b"sh")
                    || b.starts_with(b"sar")
                    || b.starts_with(b"sal")
                    || b.starts_with(b"set")
                    || b.starts_with(b"sbb")
            }
            b'x' => b.starts_with(b"xor") || b.starts_with(b"xchg") || b.starts_with(b"xadd"),
            b'c' => {
                b.starts_with(b"cmp")
                    || b.starts_with(b"cmov")
                    || b.starts_with(b"call")
                    || b.starts_with(b"cvt")
            }
            b't' => b.starts_with(b"test") || b.starts_with(b"tzcnt"),
            b'i' => b.starts_with(b"imul") || b.starts_with(b"inc") || b.starts_with(b"idiv"),
            b'd' => b.starts_with(b"dec") || b.starts_with(b"div"),
            b'n' => b.starts_with(b"neg") || b.starts_with(b"not") || b.starts_with(b"nop"),
            b'o' => b.starts_with(b"or"),
            b'p' => b.starts_with(b"push") || b.starts_with(b"pop"),
            b'r' => b.starts_with(b"ret") || b.starts_with(b"rol") || b.starts_with(b"ror"),
            b'j' => true,
            _ => false,
        };
        if fast {
            return true;
        }
    }
    const KNOWN: &[&str] = &[
        "mov",
        "lea",
        "add",
        "sub",
        "and",
        "or",
        "xor",
        "cmp",
        "test",
        "imul",
        "mul",
        "div",
        "idiv",
        "neg",
        "not",
        "inc",
        "dec",
        "shl",
        "shr",
        "sar",
        "sal",
        "rol",
        "ror",
        "adc",
        "sbb",
        "set",
        "cmov",
        "push",
        "pop",
        "call",
        "ret",
        "jmp",
        "j",
        "nop",
        "cqto",
        "cltq",
        "cdq",
        "cwtl",
        "cltd",
        "cwtd",
        "cwd",
        "cqo",
        "cdqe",
        "bswap",
        "bt",
        "bsf",
        "bsr",
        "popcnt",
        "lzcnt",
        "tzcnt",
        "xchg",
        "cmpxchg",
        "xadd",
        "lock",
        "leave",
        "endbr64",
        "ud2",
        "int3",
        "hlt",
        "pause",
        "prefetch",
        "cvt",
        "vm",
        "movs",
        "movz",
        "andn",
        "bls",
        "sh",
        "rc",
        "adcx",
        "adox",
        "mulx",
        "rdtsc",
        "cpuid",
        "syscall",
        "sfence",
        "lfence",
        "mfence",
        "vzeroupper",
        "vzeroall",
        "xgetbv",
    ];
    KNOWN.iter().any(|k| t.starts_with(k))
        // The central implicit-operand oracle doubles as a knowledge base:
        // any instruction whose implicit register contract it knows is as
        // trustworthy as the explicit list (this is what lets `cltd` —
        // emitted for every 32-bit division — take the precise path).
        || implicit_reg_refs(t.trim_ascii().as_bytes()) != 0
}

impl FileLiveness {
    /// Content key for the per-function liveness memo.
    ///
    /// The solver reads the function's *text* (markers, labels, jump tables,
    /// mnemonics) and its per-line [`LineInfo`].  Folding both in makes a hit
    /// observationally identical to a re-solve: the key covers every input.
    ///
    /// Hashing the whole `LineInfo` is not belt-and-braces.  `LineInfo` is
    /// **not** a pure function of the text: `pinned_volatile_access` and the
    /// `popcnt`/`bmi` folds in `passes/mod.rs` force `has_indirect_mem`,
    /// `rbp_offset` and `kind` on lines whose text they did not touch.  The
    /// solver's store/load modelling reads exactly those fields, so a key
    /// built from text + `pinned` + `reg_refs` alone is sound only for as
    /// long as every such mutation also happens to set `pinned`.  Folding
    /// [`LineInfo::digest_words`] — an exhaustively-destructured lossless
    /// packing — makes that coincidence unnecessary, and keeps the key
    /// complete when a field is added (the destructure fails to compile
    /// otherwise, exactly like the old `Hash` derive did).
    ///
    /// Two independent accumulators: an accidental hit then needs a collision
    /// in both 64-bit halves.  The text half rides on the store's cached
    /// per-line digest rather than a per-call re-hash.
    fn hash_function(
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) -> (u64, u64) {
        // splitmix64 finalizer: full 64-bit avalanche for the returned key.
        #[inline]
        fn mix(mut z: u64) -> u64 {
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        // Order-sensitive fold of one line digest into an accumulator.
        #[inline]
        fn fold(h: u64, w: u64) -> u64 {
            (h.rotate_left(5) ^ w).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        }
        let len = end.saturating_sub(start);
        let mut h1 = 0x243f_6a88_85a3_08d3u64 ^ len as u64;
        let mut h2 = 0x1319_8a2e_0370_7344u64 ^ (len as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        // Slice once so the per-line `LineInfo` access is a bounds-check-free
        // iterator step (the indexed form re-checked `n < infos.len()` every
        // iteration under the fastbuild profile).
        for (info, n) in infos[start..end].iter().zip(start..) {
            // The text half of the digest comes from the store's per-line
            // cache: it covers the raw stored text, which `trimmed` is a
            // pure prefix-slice of (`trim_start` is folded as part of the
            // `digest_words` packing), so the pair (raw-text digest,
            // packed LineInfo words) separates at least every pair the
            // old (trimmed-text, LineInfo) fold separated.  Re-hashing
            // the string per call was 4% of all instructions on a
            // 40-function TU; the `LineInfo` half used to go through a
            // non-inlined `FxHasher` derive walk that itself cost 1.57%
            // — `digest_words` is the same information as a LOSSLESS
            // three-word packing, folded inline.
            let [w0, w1, w2] = info.digest_words();
            // One order-sensitive chain over the four words; the
            // rotate+multiply fold keeps position sensitivity, the two
            // accumulators plus differing seeds keep 128 bits of state —
            // the same diffusion structure as the previous
            // (FxHasher-folded word, word ^ const) pair.
            let w = fold(fold(fold(w0, w1), w2), store.line_digest(n));
            h1 = fold(h1, w);
            h2 = fold(h2, w ^ 0xa409_3822_299f_31d0);
        }
        (mix(h1), mix(h2))
    }

    /// Compute liveness for every function in the file.
    #[expect(clippy::needless_range_loop)]
    pub(super) fn new(store: &LineStore, infos: &[LineInfo]) -> Self {
        #[cfg(test)]
        LIVENESS_BUILDS.with(|count| count.set(count.get() + 1));
        let len = store.len();
        let mut lv = FileLiveness {
            live_out: vec![ALL; len],
            live_in: vec![ALL; len],
            known: vec![false; len],
            structural: vec![false; len],
            effect: vec![(0, 0); len],
            func_of: vec![None; len],
            textual_ret: FxHashSet::default(),
        };
        let mut i = 0;
        while i < len {
            if !infos[i].is_cfi_start() {
                i += 1;
                continue;
            }
            let mut end = len;
            for n in i + 1..len {
                if infos[n].is_cfi_end() {
                    end = n;
                    break;
                }
            }
            for slot in &mut lv.func_of[i..end] {
                *slot = Some((i as u32, end as u32));
            }
            lv.analyse_function(store, infos, i, end, None);
            i = end.max(i + 1);
        }
        lv
    }

    /// Live family set immediately after line `idx` in the program that
    /// results from DELETING the definitions found on `lo..=hi` for the
    /// families in `mask` (reads and writes both neutralised, see
    /// `analyse_function`).  `None` when the enclosing function is not
    /// analysable.
    ///
    /// This is the question `compare_branch`'s fusion gate actually means by
    /// "the family is dead after the fused jump".  Plain [`live_after`] is a
    /// whole-program property of the CURRENT text, and it counts reads made
    /// by the boolean sequence itself -- the extension's narrow source read
    /// and the test -- which this very fusion deletes.  Across a loop's back
    /// edge that is not a local nuisance but a loop-carried bit: the rotated
    /// loop
    ///
    /// ```text
    /// .LBB4:
    ///     movzbl (%rdi,%rdx), %eax
    ///     addq %rax, %rsi
    ///     addq $1, %rdx
    ///     cmpq $2044, %rdx
    ///     setl %r8b            ; <- dropped
    ///     movzbl %r8b, %r8d    ; <- dropped (reads %r8b)
    ///     testb %r8b, %r8b     ; <- dropped (reads %r8b)
    ///     jne .LBB4            ; <- fused to `jl .LBB4`
    /// .LBB5:
    ///     movq %rsi, %rax
    /// ```
    ///
    /// has `%r8` live at the `jne` only because the back edge re-enters the
    /// header, which is above the sequence: the loop-carried bit exists
    /// solely through the two reads that the fusion removes.  The exact
    /// question ("is `%r8` observed after the fused jump on any path that
    /// does not go through the sequence?") answers dead, and the fusion
    /// fires.
    ///
    /// Soundness: `analyse_function` drops the span's writes too, so nothing
    /// is assumed dead that a surviving path could still observe.  The
    /// caller must have established that the span is straight-line,
    /// unconditionally executed between the producer and the jump, and
    /// contains every mention of the mask families that it deletes
    /// (`compare_branch` re-verifies mentionedness of its transparent lines
    /// against the final carrier mask; labels are never transparent there).
    /// Exact saved state of one function's span: everything a span-bounded
    /// re-derivation (`reanalyse_with`, `refresh_span`, `refresh_at`) can
    /// write.  Restoring it reproduces the pre-probe `FileLiveness` bit for
    /// bit: the five per-line arrays are only ever published into
    /// `start..end` by those paths, `textual_ret` only gains or loses
    /// `start`, and `func_of` is written exclusively by `new()`.
    pub(super) fn snapshot_function(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) -> Option<FnSpanSnapshot> {
        let (start, end) = self.recorded_function(store, infos, idx)?;
        Some(FnSpanSnapshot {
            start,
            end,
            live_out: self.live_out[start..end].to_vec(),
            live_in: self.live_in[start..end].to_vec(),
            known: self.known[start..end].to_vec(),
            structural: self.structural[start..end].to_vec(),
            effect: self.effect[start..end].to_vec(),
            textual_ret: self.textual_ret.contains(&start),
        })
    }

    /// Restore a span saved by [`FileLiveness::snapshot_function`].
    pub(super) fn restore_function(&mut self, snap: &FnSpanSnapshot) {
        let (start, end) = (snap.start, snap.end);
        self.live_out[start..end].copy_from_slice(&snap.live_out);
        self.live_in[start..end].copy_from_slice(&snap.live_in);
        self.known[start..end].copy_from_slice(&snap.known);
        self.structural[start..end].copy_from_slice(&snap.structural);
        self.effect[start..end].copy_from_slice(&snap.effect);
        if snap.textual_ret {
            self.textual_ret.insert(start);
        } else {
            self.textual_ret.remove(&start);
        }
    }

    pub(super) fn live_after_dropping_region(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        lo: usize,
        hi: usize,
        mask: u16,
    ) -> Option<u16> {
        if idx >= self.known.len() || lo > hi || hi >= self.known.len() {
            return None;
        }
        // Probe in place, then restore the saved span exactly.  The probe
        // used to clone all seven whole-file vectors to protect the
        // caller's (unmodified-program) solution — O(file) copies for a
        // question about ONE function.  See `snapshot_function` for the
        // write-set argument.
        let snap = self.snapshot_function(store, infos, idx)?;
        self.reanalyse_with(store, infos, snap.start, snap.end, Some((lo, hi, mask)));
        let answer = self.known[idx].then(|| self.live_out[idx]);
        self.restore_function(&snap);
        answer
    }

    /// `Some(true)` when `fam` may be read after line `idx`, `Some(false)` when
    /// it is provably dead, `None` when the enclosing function was not
    /// analysable.
    pub(super) fn live_after(&self, idx: usize, fam: RegId) -> Option<bool> {
        if fam > REG_GP_MAX || idx >= self.known.len() || !self.known[idx] {
            return None;
        }
        Some(self.live_out[idx] & (1u16 << fam) != 0)
    }

    /// Whether lines `a` and `b` lie in the same analysed function.
    pub(super) fn same_function(&self, a: usize, b: usize) -> bool {
        matches!((self.func_of.get(a), self.func_of.get(b)),
            (Some(Some(x)), Some(Some(y))) if x == y)
    }

    /// The families whose solution can change if line `idx` is deleted, when
    /// that set is exactly the line's reads; `None` when only a full
    /// [`FileLiveness::refresh_at`] is exact.
    ///
    /// Liveness is a per-family gen/kill problem.  Deleting a straight-line
    /// instruction (control flow, labels and the CFG are untouched, so the
    /// solver's convergence and the jump-table resolution are too) turns its
    /// transfer function into the identity.  A family it writes and that is
    /// dead after it had live-in 0 (killed) and keeps live-in 0 (passes the
    /// dead live-out through), so its solution is unchanged everywhere; a
    /// family it reads may only shrink; every other family is untouched.
    /// That needs: the function analysed, every written family dead after
    /// the line in THIS solution (a deletion justified by a syntactic proof
    /// while the dataflow still models the family live could grow it), and
    /// the return liveness taken from the prologue markers (the textual
    /// tail-block detector reads instruction text a deletion can change).
    pub(super) fn deletion_changes_only_reads(&self, idx: usize) -> Option<u16> {
        if !*self.known.get(idx)? {
            return None;
        }
        let (start, _) = (*self.func_of.get(idx)?)?;
        if self.textual_ret.contains(&(start as usize)) {
            return None;
        }
        let (reads, writes) = self.effect[idx];
        (self.live_out[idx] & writes == 0).then_some(reads)
    }

    /// Recompute the liveness of the function containing `idx` after a pass
    /// rewrote or deleted a line inside it. Cheaper than rebuilding the file
    /// and mandatory for correctness: a transform can extend the live range of
    /// the register it substitutes in, so a later query in the same pass must
    /// not see stale data.
    #[expect(clippy::needless_range_loop)]
    pub(super) fn refresh_at(&mut self, store: &LineStore, infos: &[LineInfo], idx: usize) {
        // The bounds recorded by `new` stay valid as long as the delimiting
        // CFI lines are intact (no pass edits them); re-derive otherwise.
        if let Some((start, end)) = self.recorded_function(store, infos, idx) {
            self.reanalyse(store, infos, start, end);
            return;
        }
        let len = store.len();
        let mut start = None;
        for n in (0..=idx.min(len.saturating_sub(1))).rev() {
            if infos[n].is_nop() {
                continue;
            }
            if infos[n].is_cfi_start() {
                start = Some(n);
                break;
            }
            if infos[n].is_cfi_end() {
                return;
            }
        }
        let Some(start) = start else { return };
        let mut end = len;
        for n in start + 1..len {
            if infos[n].is_cfi_end() {
                end = n;
                break;
            }
        }
        self.reanalyse(store, infos, start, end);
    }

    /// Forget and recompute the function `[start, end)`.
    fn reanalyse(&mut self, store: &LineStore, infos: &[LineInfo], start: usize, end: usize) {
        self.reanalyse_with(store, infos, start, end, None);
    }

    /// [`FileLiveness::reanalyse`] with the optional `neutralise` override of
    /// `analyse_function` (used by the region-dropping query).
    fn reanalyse_with(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
        neutralise: Option<(usize, usize, u16)>,
    ) {
        for n in start..end {
            self.known[n] = false;
            self.live_out[n] = ALL;
            self.live_in[n] = ALL;
            self.structural[n] = false;
            self.effect[n] = (0, 0);
        }
        self.textual_ret.remove(&start);
        self.analyse_function(store, infos, start, end, neutralise);
    }

    /// The recorded `(start, end)` of the function containing `idx`, if its
    /// delimiting CFI lines are still in place: the same half-open range
    /// `relay_and_lea::function_range` rescans for (no pass creates or
    /// deletes CFI lines; the intact check fails closed if one ever does).
    pub(super) fn recorded_function(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) -> Option<(usize, usize)> {
        let (start, end) = (*self.func_of.get(idx)?)?;
        let (start, end) = (start as usize, end as usize);
        let intact = |n: usize, what: &str| {
            !infos[n].is_nop() && infos[n].trimmed(store.get(n)).starts_with(what)
        };
        (start < store.len()
            && intact(start, ".cfi_startproc")
            && (end == store.len() || (end < store.len() && intact(end, ".cfi_endproc"))))
        .then_some((start, end))
    }

    /// Bring the solution up to date after a pass edited ONLY lines in
    /// `lo..=hi` (rewrote, deleted or re-typed them) -- the incremental form
    /// of `refresh_at`, identical in result.
    ///
    /// A peephole pass folds thousands of times per large function, and a
    /// whole-function re-analysis per fold made such passes quadratic
    /// (gcc.c-torture/compile 20001226-1: 8192 compare-and-branch pairs ran
    /// for minutes in `fuse_load_into_alu`).  Most edits are local:
    ///
    /// * when the span is straight-line code both before and after the edit
    ///   (no line of it is structural, now or at the last analysis),
    ///   the CFG and every other line's effect are unchanged;
    /// * the span is then re-derived backwards from the live-in of the first
    ///   real line after it, which the edit did not touch;
    /// * the splice is then a fixpoint of the edited function whenever the
    ///   span's new entry live-in equals the old one (no predecessor's
    ///   live-out changes; nothing can branch into the span, it has no
    ///   labels).  It is the LEAST fixpoint -- the exact analysis -- when in
    ///   addition no register live after the span lost an upward-exposed
    ///   read in it: with `gen` the span's composite read set (its live-in
    ///   for an empty live-out), `(gen_old & !gen_new) & anchor == 0`
    ///   (minus registers the code right after the span reads first: those
    ///   are live at the anchor in every solution).
    ///   Liveness is a per-register problem; for a register outside the
    ///   anchor set, or one whose composite transfer only grew, the least
    ///   solution N of the edited function also satisfies the OLD
    ///   equations, so the old (least) solution is below N, and it equals
    ///   the splice everywhere outside the span.  A register failing the
    ///   test is one a loop might have kept live only through the removed
    ///   read (entry equality alone misses it: the register then passes
    ///   straight through the span and stays live);
    /// * the one intended difference: a full re-analysis that would not
    ///   converge within its round cap leaves the function unknown, while
    ///   the splice keeps the (exact) solution.
    ///
    /// Anything else (a structural line involved, a jump-table dispatch
    /// within reach, a `ret` read set inferred from text, either check
    /// failing) falls back to `refresh_at`.
    pub(super) fn refresh_span(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        lo: usize,
        hi: usize,
    ) {
        if !self.try_refresh_span(store, infos, lo, hi) {
            self.refresh_at(store, infos, lo);
        } else if verify_span_refresh() {
            self.assert_matches_full_analysis(store, infos, lo, hi);
        }
    }

    /// `CCC_VERIFY_LIVENESS_SPAN=1`: after every spliced refresh, recompute
    /// the function from scratch and abort on any difference.  The
    /// exactness argument of `refresh_span` is checked this way over whole
    /// corpora (torture suites, regression tests); it costs a full analysis
    /// per edit, so it is off by default.
    fn assert_matches_full_analysis(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        lo: usize,
        hi: usize,
    ) {
        let Some((start, end)) = self.recorded_function(store, infos, lo) else {
            return;
        };
        let mut fresh = FileLiveness {
            live_out: self.live_out.clone(),
            live_in: self.live_in.clone(),
            known: self.known.clone(),
            structural: self.structural.clone(),
            effect: self.effect.clone(),
            func_of: Vec::new(),
            textual_ret: FxHashSet::default(),
        };
        fresh.reanalyse(store, infos, start, end);
        for n in start..end {
            let spliced = (
                self.known[n],
                self.live_in[n],
                self.live_out[n],
                self.effect[n],
            );
            let full = (
                fresh.known[n],
                fresh.live_in[n],
                fresh.live_out[n],
                fresh.effect[n],
            );
            // Where the full analysis gives up (round cap), the splice may
            // legitimately stay known; everywhere else it must agree.
            assert!(
                !fresh.known[n] || spliced == full,
                "refresh_span({lo}, {hi}) diverged from a full analysis at line {n} \
                 `{}`: spliced (known, in, out) = {spliced:?}, full = {full:?}",
                infos[n].trimmed(store.get(n))
            );
        }
    }

    fn try_refresh_span(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        lo: usize,
        hi: usize,
    ) -> bool {
        if lo > hi || hi >= store.len() {
            return false;
        }
        let Some((start, end)) = self.recorded_function(store, infos, lo) else {
            return false;
        };
        if hi >= end || lo <= start || self.textual_ret.contains(&start) {
            return false;
        }
        if (lo..=hi).any(|p| self.structural[p] || infos[p].structural) {
            return false;
        }
        let real = |n: usize| !infos[n].is_steppable();
        // A call's read set comes from the marker window after it
        // (`call_site_reads`, `CALL_MARKER_WINDOW` lines), so an edit there
        // may change it.  The exact test: recompute it on the edited text
        // and compare with the recorded transfer function (its writes and
        // CFG edges depend only on the call's own text, which a span never
        // contains -- calls are structural).  Calls are never spliced, and
        // every splice re-checks this, so the recorded read set is the one
        // of the current text.  A blanket refusal here made every fold in
        // the few lines after a call fall back to a whole-function
        // re-analysis: quadratic compile time in call-dense functions
        // (2000 `printf` calls in one `main`: 42 s at -O0).
        if (lo.saturating_sub(CALL_MARKER_WINDOW).max(start)..lo).any(|p| {
            !infos[p].is_nop()
                && infos[p].kind == LineKind::Call
                && (!self.known[p] || self.effect[p].0 != call_site_reads(store, infos, p))
        }) {
            return false;
        }
        // The first real line after the span, whose live-in anchors the
        // re-derivation.  A jump-table dispatch matches its three preceding
        // real lines, so one that close could see the span.
        let mut after = (hi + 1..end).filter(|&n| real(n));
        let Some(succ) = after.next() else {
            return false;
        };
        if !self.known[succ] {
            return false;
        }
        if std::iter::once(succ)
            .chain(after.take(2))
            .any(|n| infos[n].kind == LineKind::JmpIndirect)
        {
            return false;
        }

        let anchor = self.live_in[succ];
        // Walk the span backwards once, deriving the new solution from the
        // anchor and both composite read sets (`gen`, live-in for an empty
        // live-out): the new one from the edited text, the old one from the
        // effects recorded at the last analysis.
        let mut fresh: Vec<(usize, u16, u16, u16, u16)> = Vec::with_capacity(hi - lo + 1);
        let mut out = anchor;
        let mut gen_new = 0u16;
        let mut gen_old = 0u16;
        let mut old_entry = anchor;
        for p in (lo..=hi).rev() {
            if real(p) {
                let t = infos[p].trimmed(store.get(p));
                let eff = Self::straight_effect(infos, p, t);
                let inn = eff.reads | (out & !eff.writes);
                fresh.push((p, out, inn, eff.reads, eff.writes));
                out = inn;
                gen_new = eff.reads | (gen_new & !eff.writes);
            }
            if self.known[p] {
                let (reads, writes) = self.effect[p];
                gen_old = reads | (gen_old & !writes);
                old_entry = self.live_in[p];
            }
        }
        // A register the straight run after the span reads before writing
        // is live at the anchor in EVERY solution, whatever the span does,
        // so losing a read of it in the span cannot matter.  (Bounded: this
        // only sharpens a sufficient condition.)
        let mut gen_after = 0u16;
        let mut killed_after = 0u16;
        for n in (succ..end).filter(|&n| real(n)).take(8) {
            if infos[n].structural {
                break;
            }
            let eff = Self::straight_effect(infos, n, infos[n].trimmed(store.get(n)));
            gen_after |= eff.reads & !killed_after;
            killed_after |= eff.writes;
        }
        if out != old_entry || (gen_old & !gen_new & anchor & !gen_after) != 0 {
            return false;
        }
        for p in lo..=hi {
            self.known[p] = false;
            self.live_out[p] = ALL;
            self.live_in[p] = ALL;
            self.effect[p] = (0, 0);
        }
        for (p, lo_out, lo_in, reads, writes) in fresh {
            self.known[p] = true;
            self.live_out[p] = lo_out;
            self.live_in[p] = lo_in;
            self.effect[p] = (reads, writes);
        }
        true
    }

    /// `true` when every `ret` in the range is preceded, inside its own tail
    /// block, by a write of `%rax` and by no mention of `%rdx`: the signature
    /// of a function returning one integer in the accumulator.
    pub(super) fn returns_in_rax_only(
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) -> bool {
        let mut saw_ret = false;
        for n in start..end {
            if infos[n].is_nop() || infos[n].kind != LineKind::Ret {
                continue;
            }
            saw_ret = true;
            let mut writes_rax = false;
            let mut k = n;
            while k > start {
                k -= 1;
                if super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k))) {
                    continue;
                }
                if matches!(
                    infos[k].kind,
                    LineKind::Label
                        | LineKind::Jmp
                        | LineKind::CondJmp
                        | LineKind::JmpIndirect
                        | LineKind::Call
                ) {
                    break;
                }
                if infos[k].reg_refs & RDX != 0 {
                    // A pure `movq %rax, %rdx` in the epilogue is the dead
                    // duplicate the return-value materialisation leaves behind:
                    // an i128 return writes two DIFFERENT halves, never a copy
                    // of the accumulator. Anything else that touches %rdx here
                    // may be the high half, so the register stays live.
                    let tk = infos[k].trimmed(store.get(k));
                    if tk != "movq %rax, %rdx" {
                        return false;
                    }
                    continue;
                }
                if get_dest_reg(&infos[k]) == 0 {
                    writes_rax = true;
                }
            }
            if !writes_rax {
                return false;
            }
        }
        saw_ret
    }

    /// The prologue's `# LCCC_RET_RDX 0|1` marker (see `prologue.rs`):
    /// whether `%rdx` is read by this function's `ret`s. `Some` (either way)
    /// is authoritative; `None` means absent or illegible — the caller falls
    /// back to tail-block inference. The scan stops at the first `ret`: like
    /// its `# LCCC_RET_XMM` sibling, the marker precedes the body.
    pub(super) fn ret_rdx_marker(
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) -> Option<bool> {
        for n in start..end {
            if infos[n].is_nop() {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            if let Some(m) = t.strip_prefix("# LCCC_RET_RDX ") {
                match m.trim_ascii() {
                    "1" => return Some(true),
                    "0" => return Some(false),
                    _ => {} // illegible: keep looking, then fall back
                }
            }
            if infos[n].kind == LineKind::Ret {
                break;
            }
        }
        None
    }

    /// The prologue's `# LCCC_RET_RAX 0|1` marker (see `prologue.rs`):
    /// whether `%rax` is read by this function's `ret`s at all. Void and
    /// pure-SSE/x87-returning functions publish 0; every integer, pointer,
    /// struct (in-register or sret), and I128 return publishes 1. `Some`
    /// (either way) is authoritative; `None` means absent (hand-written
    /// fragments, unit tests) — the caller keeps the conservative
    /// RET_LIVE. Same scan contract as `ret_rdx_marker`.
    pub(super) fn ret_rax_marker(
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) -> Option<bool> {
        for n in start..end {
            if infos[n].is_nop() {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            if let Some(m) = t.strip_prefix("# LCCC_RET_RAX ") {
                match m.trim_ascii() {
                    "1" => return Some(true),
                    "0" => return Some(false),
                    _ => {} // illegible: keep looking, then fall back
                }
            }
            if infos[n].kind == LineKind::Ret {
                break;
            }
        }
        None
    }

    /// `neutralise` (see [`FileLiveness::live_after_dropping_region`]): when
    /// `Some((lo, hi, mask))`, the lines `lo..=hi` contribute NEITHER reads
    /// nor writes for `mask`'s families -- the analysis then answers
    /// liveness questions for the program that would exist if those
    /// definitions had been deleted.  Everything else is unchanged; the
    /// store/load, call and `ret` modelling never consults the mask.
    fn analyse_function(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
        neutralise: Option<(usize, usize, u16)>,
    ) {
        /// One line's cached answers, packed so a whole function caches in
        /// ONE vector instead of five (the five `.to_vec()` calls were 565k
        /// heap blocks / 211 MB per profiling TU — the largest allocation
        /// site in this pass after the solver itself).
        #[derive(Clone, Copy)]
        struct CachedLine {
            live_out: u16,
            live_in: u16,
            effect: (u16, u16),
            known: bool,
            structural: bool,
        }
        type CachedFnLiveness = (Vec<CachedLine>, bool);
        /// One generation of the memo for one compilation thread: the entries
        /// and their total line count, so the rotation test is O(1).  The
        /// bounds (`FN_CACHE_CAP`, `FN_CACHE_LINE_BUDGET`) are module-level.
        #[derive(Default)]
        struct FnCacheGen {
            map: FxHashMap<(u64, u64), CachedFnLiveness>,
            lines: usize,
        }
        // Two generations: `.0` is current, `.1` the previous one.  Retention
        // is at most two generations, so the memo holds at most about twice
        // `FN_CACHE_LINE_BUDGET` lines.
        thread_local! {
            static FN_CACHE: std::cell::RefCell<(FnCacheGen, FnCacheGen)> =
                std::cell::RefCell::new((FnCacheGen::default(), FnCacheGen::default()));
        }
        let fn_len = end.saturating_sub(start);
        // Only plain re-analyses are cached: the neutralised probes
        // (`live_after_dropping_region`) are region-specific, and on the
        // 1128-function reproducer TU zero of them fired, so keying them
        // measured as noise.  If kernel TUs turn out to be probe-heavy,
        // probe caching should be revisited with a probe histogram in
        // hand (see MB-20 follow-ups).
        let cache_key = if neutralise.is_none() && fn_len >= FN_CACHE_MIN_LINES {
            let key = Self::hash_function(store, infos, start, end);
            let hit = FN_CACHE.with(|c| {
                let mut banks = c.borrow_mut();
                if !banks.0.map.contains_key(&key)
                    && let Some(promoted) = banks.1.map.remove(&key)
                {
                    let promoted_lines = promoted.0.len();
                    banks.1.lines -= promoted_lines;
                    if fn_cache_gen_full(banks.0.map.len(), banks.0.lines, promoted_lines) {
                        banks.1 = std::mem::take(&mut banks.0);
                    }
                    banks.0.lines += promoted_lines;
                    banks.0.map.insert(key, promoted);
                }
                if let Some((cached, tret)) = banks.0.map.get(&key)
                    && cached.len() == fn_len
                {
                    // Five zipped streams: equal-length slices let the
                    // compiler drop every per-element bounds check (the
                    // indexed spelling cost ~0.5B instructions per TU).
                    let lo = &mut self.live_out[start..end];
                    let li = &mut self.live_in[start..end];
                    let kn = &mut self.known[start..end];
                    let st = &mut self.structural[start..end];
                    let ef = &mut self.effect[start..end];
                    for (c, ((((lo, li), kn), st), ef)) in cached.iter().zip(
                        lo.iter_mut()
                            .zip(li.iter_mut())
                            .zip(kn.iter_mut())
                            .zip(st.iter_mut())
                            .zip(ef.iter_mut()),
                    ) {
                        *lo = c.live_out;
                        *li = c.live_in;
                        *kn = c.known;
                        *st = c.structural;
                        *ef = c.effect;
                    }
                    if *tret {
                        self.textual_ret.insert(start);
                    }
                    true
                } else {
                    false
                }
            });
            if hit {
                #[cfg(test)]
                MEMO_HITS.with(|c| c.set(c.get() + 1));
                return;
            }
            Some(key)
        } else {
            None
        };

        let solved = SOLVE_SCRATCH.with(|c| {
            self.solve_function(store, infos, start, end, neutralise, &mut c.borrow_mut())
        });
        if !solved {
            return;
        }
        if let Some(key) = cache_key {
            FN_CACHE.with(|c| {
                let mut banks = c.borrow_mut();
                // Two-generation bounded cache (`curr`, `prev`): when `curr`
                // fills, rotate `curr` into `prev` so hot entries from the
                // immediately preceding pass are promoted on hit while new
                // functions continue to be cached (neither a clear-the-world
                // cliff nor a permanent freeze after FN_CACHE_CAP functions).
                if fn_cache_gen_full(banks.0.map.len(), banks.0.lines, fn_len) {
                    banks.1 = std::mem::take(&mut banks.0);
                }
                let mut cached = Vec::with_capacity(fn_len);
                // Zipped gather for the same reason as the hit path: the
                // indexed loop re-checked five slice bounds per line.
                cached.extend(
                    self.live_out[start..end]
                        .iter()
                        .zip(self.live_in[start..end].iter())
                        .zip(self.known[start..end].iter())
                        .zip(self.structural[start..end].iter())
                        .zip(self.effect[start..end].iter())
                        .map(|((((&lo, &li), &kn), &st), &ef)| CachedLine {
                            live_out: lo,
                            live_in: li,
                            effect: ef,
                            known: kn,
                            structural: st,
                        }),
                );
                // A same-key entry can only be displaced by a 128-bit hash
                // collision (the length check above then misses); its lines
                // leave the count so `lines` stays the exact sum of entries.
                banks.0.lines += fn_len;
                if let Some((displaced, _)) = banks
                    .0
                    .map
                    .insert(key, (cached, self.textual_ret.contains(&start)))
                {
                    banks.0.lines -= displaced.len();
                }
            });
        }
    }

    /// The uncached solve of one function: build effects and successor
    /// edges, run the backward dataflow and, on convergence, publish the
    /// per-line answers into `self`.  Returns `false` when the control
    /// flow is unanalysable or the solve did not converge (the lines stay
    /// `known == false` and the caller must not cache).  All working
    /// memory comes from `scratch` — see `SolveScratch`.
    fn solve_function(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
        neutralise: Option<(usize, usize, u16)>,
        scratch: &mut SolveScratch,
    ) -> bool {
        let fn_len = end.saturating_sub(start);
        let rdx_marker = Self::ret_rdx_marker(store, infos, start, end);
        if rdx_marker.is_none() {
            self.textual_ret.insert(start);
        }
        let ret_live = match rdx_marker {
            // The prologue marker is authoritative: it was computed from the
            // signature's return classification, which no tail-block shape
            // can defeat (a shared epilogue, `%rdx` traffic that is a mere
            // read, ...). It governs ONLY the `%rdx` bit; everything else in
            // RET_LIVE (callee-saved must-restore, `%rax`) stands.
            Some(true) => RET_LIVE,
            Some(false) => RET_LIVE_RAX_ONLY,
            // No (legible) marker — hand-written fragments, unit tests: the
            // tail-block detector below.
            None => {
                if Self::returns_in_rax_only(store, infos, start, end) {
                    RET_LIVE_RAX_ONLY
                } else {
                    RET_LIVE
                }
            }
        };
        // The RAX marker governs the `%rax` bit the same way: a void or
        // pure-SSE/x87-returning function reads NOTHING from the
        // accumulator at `ret` (the SysV psABI only defines %rax's value at
        // the call boundary for integer/sret returns), so dropping it is
        // sound and unlocks the copy-folding of the RA's phi-convergence
        // copies into %rax in exactly those functions. Absent marker (or
        // illegible) keeps the conservative live bit.
        let ret_live = match Self::ret_rax_marker(store, infos, start, end) {
            Some(false) => ret_live & !RAX,
            _ => ret_live,
        };
        // ── labels ───────────────────────────────────────────────────────────
        // Hashed: every branch resolves its target here, and a linear scan
        // made the analysis O(branches x labels) -- 8192 compare-and-branch
        // pairs (gcc.c-torture/compile 20001226-1) spent minutes in string
        // compares, once per `refresh_at`.  The FIRST definition wins, as
        // with the scan it replaces.
        let nlabels = (start..end)
            .filter(|&n| !infos[n].is_nop() && infos[n].kind == LineKind::Label)
            .count();
        let mut label_index: FxHashMap<&str, usize> =
            FxHashMap::with_capacity_and_hasher(nlabels, Default::default());
        for n in start..end {
            if infos[n].is_nop() || infos[n].kind != LineKind::Label {
                continue;
            }
            if let Some(name) = infos[n].trimmed(store.get(n)).strip_suffix(':') {
                label_index.entry(name).or_insert(n);
            }
        }
        let resolve = |name: &str| -> Option<usize> { label_index.get(name).copied() };

        // ── instruction effects + successor edges ────────────────────────────
        // Successor edges live in ONE flat buffer, line `rel` owning
        // `succ_flat[succ_range[rel].0..succ_range[rel].1]`.  A `Vec` per line
        // cost a heap allocation and free for nearly every line of every
        // solve (almost all have one or two successors); the flat buffer
        // grows to its high-water mark once per solve.
        let SolveScratch {
            effects,
            succ_range,
            lines,
            succ_flat,
            live_in,
            live_out,
        } = scratch;
        effects.clear();
        effects.resize(fn_len, None);
        succ_range.clear();
        succ_range.resize(fn_len, (0, 0));
        lines.clear();
        for n in start..end {
            if super::helpers::steps_over_line(&infos[n], infos[n].trimmed(store.get(n))) {
                continue;
            }
            lines.push(n);
        }
        // Inline asm whose text can be read line by line: what it reads, and
        // the labels it may leave for.  Any other region reads everything.
        let asm_regions = asm_regions(store, infos, start, end, &resolve);

        // ── jump-table dispatch resolution ─────────────────────────────────
        // The codegen lowers dense integer switches to exactly:
        //     leaq .LjtN(%rip), %rcx
        //     movslq (%rcx,%rax,4), %rdx
        //     addq %rcx, %rdx
        //     jmpq *%rdx
        // with the table emitted INLINE as an interleaved .rodata island:
        //     .LjtN: .long .LBBa - .LjtN   (× k entries)
        // A `jmpq *` is generically unresolvable, and ONE such line marks
        // the whole function unanalysable — silently disabling every
        // liveness-gated transform in it. The sqlite_varint / glibc_memcmp
        // switch shapes lost 2–4 instructions per arm exactly this way
        // (movzbl+movq+movb chains that fold fine in jump-table-free
        // functions). The TABLE is static data in the same file, so the
        // successor set is exactly the table's target labels: resolving
        // it keeps the function fully analysable. Any OTHER indirect
        // jump (computed goto, tail call through a register) still
        // refuses analysis.
        let jump_table_succs: Vec<Option<Vec<usize>>> =
            Self::resolve_jump_tables(store, infos, lines, &label_index, start, end);
        succ_flat.clear();
        for (pos, &n) in lines.iter().enumerate() {
            let t = infos[n].trimmed(store.get(n));
            let next = lines.get(pos + 1).copied();
            let rel = n - start;
            let first_edge = succ_flat.len();
            let read_asm = (infos[n].kind == LineKind::InlineAsm)
                .then(|| asm_regions.iter().find(|r| (r.first..=r.last).contains(&n)))
                .flatten();
            let eff = if let Some(region) = read_asm {
                // Every line of the region carries the region's reads and
                // writes nothing, so the answer in front of the region does
                // not depend on the control flow inside it; the labels it
                // names are successors of its last line.
                succ_flat.extend(next);
                if n == region.last {
                    succ_flat.extend(region.targets.iter().filter(|&&t| Some(t) != next));
                }
                Effect {
                    reads: region.reads,
                    writes: 0,
                }
            } else {
                match Self::classify(
                    store,
                    infos,
                    n,
                    t,
                    &resolve,
                    next,
                    ret_live,
                    &jump_table_succs,
                    start,
                    succ_flat,
                ) {
                    Some(v) => v,
                    None => return false, // unanalysable control flow: leave `known` false
                }
            };
            let eff = match neutralise {
                // Deleted-definition semantics: the span is no longer an
                // observation of these families, and no longer a
                // definition either.  Both halves matter -- dropping the
                // reads models "the sequence's own consumers are gone";
                // dropping the writes is what keeps the answer SOUND
                // across a back edge (with the write still counted, the
                // region would keep killing the family on every
                // iteration, and a genuine loop-carried observation
                // below the region would be reported dead).
                Some((lo, hi, mask)) if n >= lo && n <= hi => Effect {
                    reads: eff.reads & !mask,
                    writes: eff.writes & !mask,
                },
                _ => eff,
            };
            effects[rel] = Some(eff);
            succ_range[rel] = (first_edge, succ_flat.len());
        }

        // ── backward dataflow ────────────────────────────────────────────────
        live_in.clear();
        live_in.resize(fn_len, 0);
        live_out.clear();
        live_out.resize(fn_len, 0);
        let mut changed = true;
        let mut rounds = 0;
        while changed && rounds < 64 {
            changed = false;
            rounds += 1;
            for &n in lines.iter().rev() {
                let rel = n - start;
                let Some(eff) = effects[rel] else { continue };
                let (first, past) = succ_range[rel];
                let mut out = 0u16;
                for &s in &succ_flat[first..past] {
                    out |= live_in[s - start];
                }
                // No successor: a `ret` (or falling off the end) hands the
                // return liveness to the caller. A sibling call's `jmp` --
                // the only jump `classify` leaves without an edge -- does
                // not: its effect already reads what the tail callee and
                // our caller observe, and the return-value registers are
                // the callee's to produce.
                if first == past && !matches!(infos[n].kind, LineKind::Jmp | LineKind::JmpIndirect)
                {
                    out |= ret_live; // fell off the end: be conservative
                }
                let inn = eff.reads | (out & !eff.writes);
                if out != live_out[rel] || inn != live_in[rel] {
                    live_out[rel] = out;
                    live_in[rel] = inn;
                    changed = true;
                }
            }
        }
        if changed {
            return false; // did not converge (pathological CFG): stay unknown
        }

        for &n in lines.iter() {
            self.live_out[n] = live_out[n - start];
            self.live_in[n] = live_in[n - start];
            self.known[n] = true;
            if let Some(eff) = effects[n - start] {
                self.effect[n] = (eff.reads, eff.writes);
            }
        }
        for n in start..end {
            self.structural[n] = infos[n].structural;
        }
        true
    }

    /// Whether GP family `fam` provably holds a zero upper half (bits 32..63)
    /// on entry to line `label`, on EVERY path into it: the cross-block
    /// continuation of `relay_and_lea::upper32_zero_at`, which stops at the
    /// first label it meets.
    ///
    /// A forward must-dataflow over the CFG the liveness solver itself builds
    /// (`classify` + `resolve_jump_tables`), so every control-flow shape it
    /// refuses (an unresolved indirect jump, unanalysable lines) is refused
    /// here too and the answer is "not known".  Lattice: one bit per family,
    /// meet = AND over predecessors, function entry = 0 (SysV leaves the
    /// upper halves of arguments unspecified), lines no path reaches = 0.
    /// The transfer is `upper32_zero_at`'s scan step applied forwards: the
    /// same stops (pinned, barrier other than label/jump, inline asm, odd
    /// case, implicit-register line) clear every bit; a may-write of a family
    /// (`writes_family`, explicit or implicit) sets its bit only when the
    /// write itself proves a zero upper half.
    ///
    /// Computed from the CURRENT text (memoised by the liveness memo's
    /// content key), so no peephole edit can leave a stale fact behind.
    pub(super) fn upper32_zero_into_label(
        store: &LineStore,
        infos: &[LineInfo],
        label: usize,
        fam: RegId,
    ) -> bool {
        if fam > REG_GP_MAX
            || label >= infos.len()
            || infos[label].kind != LineKind::Label
            || infos[label].pinned
        {
            return false;
        }
        if Self::fallthrough_path_refutes_upper32_zero(store, infos, label, fam) {
            return false;
        }
        let Some((start, end)) = Self::enclosing_function(store, infos, label) else {
            return false;
        };
        type UpperMemo = FxHashMap<(u64, u64), Option<Vec<u16>>>;
        /// Same bounded two-generation policy as the liveness memo.
        const UPPER_CACHE_CAP: usize = 64;
        thread_local! {
            static UPPER_CACHE: std::cell::RefCell<(UpperMemo, UpperMemo)> =
                std::cell::RefCell::new((FxHashMap::default(), FxHashMap::default()));
        }
        let key = Self::hash_function(store, infos, start, end);
        let rel = label - start;
        let bit_of = |sol: &Option<Vec<u16>>| {
            sol.as_ref()
                .is_some_and(|v| v.get(rel).is_some_and(|m| m & (1u16 << fam) != 0))
        };
        if let Some(hit) = UPPER_CACHE.with(|c| {
            let banks = c.borrow();
            banks.0.get(&key).or_else(|| banks.1.get(&key)).map(bit_of)
        }) {
            return hit;
        }
        let solution = Self::solve_upper32_zero(store, infos, start, end);
        let answer = bit_of(&solution);
        UPPER_CACHE.with(|c| {
            let mut banks = c.borrow_mut();
            if banks.0.len() >= UPPER_CACHE_CAP {
                banks.1 = std::mem::take(&mut banks.0);
            }
            banks.0.insert(key, solution);
        });
        answer
    }

    /// Exact early answer for the common failing query: in a must-analysis
    /// ONE predecessor path that kills the fact decides it.  Walk the
    /// straight-line run that falls into `label` backwards with the solver's
    /// own transfer (labels and jumps transparent): a definite kill or the
    /// function's entry refutes; a proving write, another label, or a
    /// fallthrough-free predecessor (`jmp`/`ret`/indirect jump) is
    /// inconclusive and the full solve decides.  Answers only `false`, which
    /// is always sound; it differs from the solve at most on a run the solve
    /// would find unreachable.
    fn fallthrough_path_refutes_upper32_zero(
        store: &LineStore,
        infos: &[LineInfo],
        label: usize,
        fam: RegId,
    ) -> bool {
        let bit = 1u16 << fam;
        for k in (0..label).rev() {
            let info = &infos[k];
            let t = info.trimmed(store.get(k));
            if super::helpers::steps_over_line(info, t) {
                if info.is_cfi_start() {
                    return true; // the entry: argument upper halves unspecified
                }
                continue;
            }
            // Another label, or a line that cannot fall through: this run
            // is not (or not only) the label's fallthrough predecessor.
            if matches!(
                info.kind,
                LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
            ) {
                return false;
            }
            let (keep, proved) = upper32_zero_transfer(info, t);
            if proved & bit != 0 {
                return false;
            }
            if keep & bit == 0 {
                return true;
            }
        }
        false
    }

    /// `(start, end)` of the function containing line `n`, delimited exactly
    /// as `FileLiveness::new` delimits it (`.cfi_startproc` .. `.cfi_endproc`
    /// or the end of the file); `None` outside every function.
    fn enclosing_function(
        store: &LineStore,
        infos: &[LineInfo],
        n: usize,
    ) -> Option<(usize, usize)> {
        // CFI lines classify as directives (leading '.'): test the kind first.
        let is = |k: usize, d: &str| {
            infos[k].kind == LineKind::Directive
                && !infos[k].is_nop()
                && infos[k].trimmed(store.get(k)).starts_with(d)
        };
        let mut start = None;
        for k in (0..=n).rev() {
            if k != n && is(k, ".cfi_endproc") {
                return None;
            }
            if is(k, ".cfi_startproc") {
                start = Some(k);
                break;
            }
        }
        let start = start?;
        let end = (n + 1..infos.len())
            .find(|&k| is(k, ".cfi_endproc"))
            .unwrap_or(infos.len());
        Some((start, end))
    }

    /// Per-line "upper half known zero on entry" masks (relative to `start`)
    /// for one function, or `None` when its control flow is not analysable
    /// or the solve does not converge.  See `upper32_zero_into_label`.
    fn solve_upper32_zero(
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) -> Option<Vec<u16>> {
        let fn_len = end.saturating_sub(start);
        let mut label_index: FxHashMap<&str, usize> = FxHashMap::default();
        for n in start..end {
            if infos[n].is_nop() || infos[n].kind != LineKind::Label {
                continue;
            }
            if let Some(name) = infos[n].trimmed(store.get(n)).strip_suffix(':') {
                label_index.entry(name).or_insert(n);
            }
        }
        let resolve = |name: &str| -> Option<usize> { label_index.get(name).copied() };
        let lines: Vec<usize> = (start..end)
            .filter(|&n| {
                !super::helpers::steps_over_line(&infos[n], infos[n].trimmed(store.get(n)))
            })
            .collect();
        let entry = *lines.first()?;
        let jump_table_succs =
            Self::resolve_jump_tables(store, infos, &lines, &label_index, start, end);
        let mut succ_flat: Vec<usize> = Vec::with_capacity(lines.len() + lines.len() / 4);
        let mut succ_range: Vec<(usize, usize)> = vec![(0, 0); fn_len];
        let mut transfer: Vec<(u16, u16)> = vec![(ALL, 0); fn_len];
        for (pos, &n) in lines.iter().enumerate() {
            let t = infos[n].trimmed(store.get(n));
            let first = succ_flat.len();
            Self::classify(
                store,
                infos,
                n,
                t,
                &resolve,
                lines.get(pos + 1).copied(),
                RET_LIVE,
                &jump_table_succs,
                start,
                &mut succ_flat,
            )?;
            succ_range[n - start] = (first, succ_flat.len());
            transfer[n - start] = upper32_zero_transfer_cached(&infos[n], t);
        }
        // Predecessor lists (CSR over the successor edges), so each line's
        // meet reads its predecessors' CURRENT out-sets.
        let mut pred_start: Vec<u32> = vec![0; fn_len + 1];
        for &n in &lines {
            let (first, past) = succ_range[n - start];
            for &succ in &succ_flat[first..past] {
                pred_start[succ - start + 1] += 1;
            }
        }
        for rel in 0..fn_len {
            pred_start[rel + 1] += pred_start[rel];
        }
        let mut fill: Vec<u32> = pred_start[..fn_len].to_vec();
        let mut preds: Vec<u32> = vec![0; succ_flat.len()];
        for &n in &lines {
            let (first, past) = succ_range[n - start];
            for &succ in &succ_flat[first..past] {
                let slot = &mut fill[succ - start];
                preds[*slot as usize] = (n - start) as u32;
                *slot += 1;
            }
        }
        // Forward must-dataflow, Gauss-Seidel in program order: a fact
        // crosses a whole straight-line run in one sweep, so the sweep count
        // tracks loop nesting, not function length.  `out` starts at TOP and
        // only descends (greatest fixed point); only `reached` lines -- some
        // path from the entry gets there -- feed a meet.
        let entry_rel = entry - start;
        let mut inn: Vec<u16> = vec![0; fn_len];
        let mut out: Vec<u16> = vec![ALL; fn_len];
        let mut reached: Vec<bool> = vec![false; fn_len];
        let mut changed = true;
        let mut sweeps = 0;
        while changed {
            if sweeps == 64 {
                return None; // did not converge (pathological CFG): unknown
            }
            sweeps += 1;
            changed = false;
            for &n in &lines {
                let rel = n - start;
                // SysV: argument upper halves are unspecified at the entry.
                let mut any = rel == entry_rel;
                let mut meet = if any { 0 } else { ALL };
                for &p in &preds[pred_start[rel] as usize..pred_start[rel + 1] as usize] {
                    if reached[p as usize] {
                        any = true;
                        meet &= out[p as usize];
                    }
                }
                if !any {
                    continue;
                }
                let (keep, proved) = transfer[rel];
                let new_out = (meet & keep) | proved;
                if !reached[rel] || meet != inn[rel] || new_out != out[rel] {
                    reached[rel] = true;
                    inn[rel] = meet;
                    out[rel] = new_out;
                    changed = true;
                }
            }
        }
        for (rel, slot) in inn.iter_mut().enumerate() {
            if !reached[rel] {
                *slot = 0;
            }
        }
        #[cfg(test)]
        UPPER_SWEEPS.with(|c| c.set(sweeps));
        Some(inn)
    }

    /// Resolve the codegen's jump-table dispatch sites to their static
    /// successor sets. Returns a slot per line (`None` = not a resolved
    /// table dispatch). See the call site for the shape being matched:
    /// `leaq .LjtN(%rip), %rB; movslq (%rB,%rA,4), %rT; addq %rB, %rT;
    /// jmpq *%rT` with the table `.LjtN: .long target - .LjtN …` emitted
    /// inline. The match is deliberately exact: a computed goto or a tail
    /// call sharing a register with a nearby leaq must NOT be mistaken
    /// for a table (that would invent CFG edges).
    fn resolve_jump_tables(
        store: &LineStore,
        infos: &[LineInfo],
        lines: &[usize],
        label_index: &FxHashMap<&str, usize>,
        start: usize,
        end: usize,
    ) -> Vec<Option<Vec<usize>>> {
        // The overwhelmingly common function has NO indirect jump, and
        // `classify` only consults this table for `JmpIndirect` lines (via
        // `.get()`, which answers `None` on the empty slice exactly as the
        // dense all-`None` table did).  Skipping the dense per-line vector
        // for those functions removes what DHAT measured as the largest
        // allocation site of the whole backend on the profiling TU:
        // 183k blocks / 372 MB of zero-initialised `Option<Vec>` entries
        // that no lookup ever touched.
        if !lines
            .iter()
            .any(|&n| infos[n].kind == LineKind::JmpIndirect)
        {
            return Vec::new();
        }
        let mut out: Vec<Option<Vec<usize>>> = vec![None; end.saturating_sub(start)];
        let resolve = |name: &str| -> Option<usize> { label_index.get(name).copied() };
        for (pos, &n) in lines.iter().enumerate() {
            if infos[n].kind != LineKind::JmpIndirect {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            // `jmpq *%rT` — take the target register. The thunk-extern
            // retpoline form `jmp __x86_indirect_thunk_rT` dispatches
            // through the same %rT (read by the thunk symbol): the
            // leaq/movslq/addq staging chain above it is identical, so
            // the table resolves exactly the same way. Without this arm a
            // PIC switch under retpoline made the whole function
            // unanalysable (JmpIndirect with unknown successors), losing
            // every liveness-driven peephole in it. Both spellings are
            // normalised to the bare register name for the comparisons
            // with the staging lines' `%rT` operands.
            let target_bare: &str = if let Some(r) = t.strip_prefix("jmpq *") {
                r.trim_start_matches('%')
            } else if let Some(name) = t.strip_prefix("jmp __x86_indirect_thunk_") {
                name
            } else {
                continue;
            };
            // The two preceding instructions must be `addq %rB, %rT` and
            // `movslq (%rB,%rA,4), %rT`.
            let (Some(add_idx), Some(mov_idx)) = (pos.checked_sub(1), pos.checked_sub(2)) else {
                continue;
            };
            let add_t = infos[lines[add_idx]].trimmed(store.get(lines[add_idx]));
            let mov_t = infos[lines[mov_idx]].trimmed(store.get(lines[mov_idx]));
            let Some(add_rest) = add_t.strip_prefix("addq ") else {
                continue;
            };
            let (Some(base_reg), Some(add_dst)) = (
                add_rest.split_once(',').map(|(b, _)| b.trim_ascii()),
                add_rest.split_once(',').map(|(_, d)| d.trim_ascii()),
            ) else {
                continue;
            };
            if add_dst.trim_start_matches('%') != target_bare {
                continue;
            }
            let Some(mov_rest) = mov_t.strip_prefix("movslq (") else {
                continue;
            };
            let Some(close) = mov_rest.find(')') else {
                continue;
            };
            let mem = &mov_rest[..close];
            // `(%rB,%rA,4)` — the base must match the addq base.
            let parts: Vec<&str> = mem.split(',').map(str::trim_ascii).collect();
            if parts.len() != 3 || parts[0] != base_reg || parts[2] != "4" {
                continue;
            }
            let mov_dst = mov_rest[close + 1..].trim_start_matches(',').trim_ascii();
            if mov_dst.trim_start_matches('%') != target_bare {
                continue;
            }
            // Third preceding instruction: `leaq .LjtN(%rip), %rB`.
            let Some(lea_idx) = pos.checked_sub(3) else {
                continue;
            };
            let lea_t = infos[lines[lea_idx]].trimmed(store.get(lines[lea_idx]));
            let Some(lea_rest) = lea_t.strip_prefix("leaq ") else {
                continue;
            };
            let Some((lea_op, lea_dst)) = lea_rest.split_once(',') else {
                continue;
            };
            if lea_dst.trim_ascii() != base_reg {
                continue;
            }
            let lea_op = lea_op.trim_ascii();
            let Some(table_name) = lea_op.strip_suffix("(%rip)") else {
                continue;
            };
            // Find the table label in the function and parse its .long
            // entries. The table island sits between the dispatch and the
            // arm labels (interleaved sections), still inside
            // [start, end). Directives are skipped from `lines`, so scan
            // the RAW line range for the label.
            let Some(tbl_line) = resolve(table_name) else {
                continue;
            };
            let mut targets: Vec<usize> = Vec::new();
            let mut k = tbl_line + 1;
            let mut ok = true;
            while k < end {
                if infos[k].is_nop() {
                    k += 1;
                    continue;
                }
                let kt = infos[k].trimmed(store.get(k));
                if let Some(entry) = kt.strip_prefix(".long ") {
                    // `.long .LBBa - .LjtN` (the subtractand is the table
                    // itself; accept the plain `.long .LBBa` spelling too).
                    let sym = entry.split('-').next().unwrap_or(entry).trim_ascii();
                    match resolve(sym) {
                        Some(idx) => targets.push(idx),
                        None => {
                            ok = false;
                        }
                    }
                    if !ok {
                        break;
                    }
                    k += 1;
                    continue;
                }
                if kt.starts_with('.') {
                    // `.section .text`, `.align` — table island boundaries.
                    break;
                }
                break;
            }
            if !ok || targets.is_empty() {
                continue;
            }
            out[n - start] = Some(targets);
        }
        out
    }

    /// The effect of line `n`, with its successor lines appended to `edges`
    /// (none for `ret`, which the solver then treats as reading `ret_live`),
    /// or `None` when the control transfer cannot be resolved -- `edges` is
    /// then garbage, and the caller abandons the function.
    /// The effect of a `jmp` that leaves the function: a sibling call when
    /// the codegen marked it (`tail_call_marked`), modeled as a call plus a
    /// return -- no successor in this function, reads what the call reads
    /// (its marker block and its own operands) plus the callee-saved
    /// registers a return owes the caller (the return-value registers are
    /// the tail callee's to produce). `None` (unanalysable) otherwise.
    fn tail_call_effect(store: &LineStore, infos: &[LineInfo], n: usize) -> Option<Effect> {
        tail_call_marked(store, infos, n).then(|| Effect {
            reads: call_site_reads(store, infos, n) | TAIL_CALL_LIVE,
            writes: 0,
        })
    }

    fn classify(
        store: &LineStore,
        infos: &[LineInfo],
        n: usize,
        t: &str,
        resolve: &dyn Fn(&str) -> Option<usize>,
        next: Option<usize>,
        ret_live: u16,
        jump_table_succs: &[Option<Vec<usize>>],
        jump_table_base: usize,
        edges: &mut Vec<usize>,
    ) -> Option<Effect> {
        let mentioned = infos[n].reg_refs;

        // Opaque assembly is classified directly; `pinned` is deliberately
        // not a second test because every InlineAsm line is already opaque.
        // `matches!` avoids invoking derived `LineKind::eq` in this hot path
        // (one check per line per solve).
        if matches!(infos[n].kind, LineKind::InlineAsm) {
            edges.extend(next);
            return Some(Effect {
                reads: ALL,
                writes: 0,
            });
        }

        match infos[n].kind {
            LineKind::Label => {
                edges.extend(next);
                Some(Effect {
                    reads: 0,
                    writes: 0,
                })
            }
            LineKind::Ret => Some(Effect {
                reads: ret_live | mentioned,
                writes: 0,
            }),
            LineKind::Call => {
                // Call-site reads: see `call_site_reads` (the shared
                // authority for the LCCC_CALL_ARGS / LCCC_VA_CALL /
                // LCCC_CHAIN_CALL marker protocol).
                let reads = call_site_reads(store, infos, n);
                // Intra-function call target (the inline-retpoline
                // `.Lrpl_set/.Lrpl_inner` pair, local trampolines): control
                // transfers to the label, and the `ret` inside returns to
                // this call's fallthrough. `resolve` only knows labels from
                // THIS function's line range, so a successful resolution
                // proves the target is local. WITHOUT the target edge the
                // thunk blocks are unreachable in the CFG, their %r10 read
                // never propagates back to the staging write, and the
                // staged retpoline target gets dead-store-eliminated
                // (`retpoline_thunk_inline` SIGSEGV — the exact hole the
                // pre-marker every-call-reads-%r10 model had been papering
                // over; the inline-thunk shape does NOT mention %r10 at the
                // call site, the mention lives in the target block). A
                // LOCAL target's register effects are modeled exactly by
                // its own instructions through the edge, so the
                // conservative CALLER_SAVED clobber is dropped for it:
                // values live at either the target or the fallthrough stay
                // live across the call (sound — dead is never concluded),
                // while a real external callee keeps the ABI model.
                let local_target = second_token(t).and_then(resolve);
                edges.extend(next);
                match local_target {
                    Some(target) => {
                        if next != Some(target) {
                            edges.push(target);
                        }
                        Some(Effect { reads, writes: 0 })
                    }
                    None => Some(Effect {
                        reads,
                        writes: CALLER_SAVED,
                    }),
                }
            }
            LineKind::JmpIndirect => {
                // A resolved jump-table dispatch: successors are the table's
                // target labels (see `resolve_jump_tables`). The dispatch
                // registers are ordinary instructions classified on their
                // own lines; this line only reads the target register.
                let Some(Some(targets)) = jump_table_succs.get(n - jump_table_base) else {
                    return Self::tail_call_effect(store, infos, n);
                };
                edges.extend_from_slice(targets);
                Some(Effect {
                    reads: mentioned,
                    writes: 0,
                })
            }
            LineKind::Jmp => {
                let target = second_token(t)?;
                if target.starts_with('*') {
                    return Self::tail_call_effect(store, infos, n);
                }
                // A label of this function is an ordinary edge, marker or
                // not; anything else is a sibling call when the codegen
                // marked it, and an unknown transfer otherwise.
                let Some(idx) = resolve(target) else {
                    return Self::tail_call_effect(store, infos, n);
                };
                edges.push(idx);
                Some(Effect {
                    reads: mentioned,
                    writes: 0,
                })
            }
            LineKind::CondJmp => {
                let target = second_token(t)?;
                let Some(idx) = resolve(target) else {
                    // A conditional sibling call (`jne sym@PLT` carrying the
                    // `# LCCC_TAIL_CALL` block, `jump_thread::
                    // fold_conditional_tail_calls`): the taken edge leaves the
                    // function reading what the tail call reads, the
                    // fall-through continues here.  Unmarked foreign targets
                    // stay unknown.
                    let tail = Self::tail_call_effect(store, infos, n)?;
                    edges.extend(next);
                    return Some(Effect {
                        reads: tail.reads | mentioned,
                        writes: 0,
                    });
                };
                edges.extend(next);
                edges.push(idx);
                Some(Effect {
                    reads: mentioned,
                    writes: 0,
                })
            }
            _ => {
                edges.extend(next);
                Some(Self::straight_effect(infos, n, t))
            }
        }
    }

    /// Register effect of a line that is not a control transfer, label,
    /// call, `ret` or inline asm: a function of the line's own text alone,
    /// which is what lets `refresh_span` re-derive it in isolation.
    ///
    /// Memoised per thread.  The solver re-derives every line's effect on
    /// every solve, and the memo of whole functions rarely hits while a pass
    /// is editing them; re-parsing the text (prefix scans, family substring
    /// searches, the implicit-operand oracle) was a quarter of all compile
    /// instructions on large inlined functions.  The key is EVERYTHING the
    /// derivation reads -- the trimmed text, `kind` (which carries the
    /// destination `get_dest_reg` reports) and `reg_refs` -- compared for
    /// equality on a hit, so a hit is the uncached answer by construction (no
    /// hash-collision argument needed).  One entry per text: a text seen
    /// with a different `kind`/`reg_refs` (a pass re-typed the line without
    /// editing it) recomputes and replaces the entry.
    fn straight_effect(infos: &[LineInfo], n: usize, t: &str) -> Effect {
        /// Bounded memory: a full table is cleared, not frozen, so it keeps
        /// tracking the texts the current passes are producing.
        const EFFECT_MEMO_CAP: usize = 1 << 15;
        type Memo = FxHashMap<Box<str>, (LineKind, u16, Effect)>;
        thread_local! {
            static EFFECT_MEMO: std::cell::RefCell<Memo> =
                std::cell::RefCell::new(FxHashMap::default());
        }
        let info = &infos[n];
        EFFECT_MEMO.with(|memo| {
            let mut memo = memo.borrow_mut();
            if let Some(entry) = memo.get_mut(t) {
                if entry.0 == info.kind && entry.1 == info.reg_refs {
                    #[cfg(test)]
                    EFFECT_MEMO_HITS.with(|c| c.set(c.get() + 1));
                    return entry.2;
                }
                let eff = Self::straight_effect_uncached(info, t);
                *entry = (info.kind, info.reg_refs, eff);
                return eff;
            }
            let eff = Self::straight_effect_uncached(info, t);
            if memo.len() >= EFFECT_MEMO_CAP {
                memo.clear();
            }
            memo.insert(t.into(), (info.kind, info.reg_refs, eff));
            eff
        })
    }

    /// [`FileLiveness::straight_effect`] without the memo.
    fn straight_effect_uncached(info: &LineInfo, t: &str) -> Effect {
        let mentioned = info.reg_refs;
        match info.kind {
            LineKind::Push { .. } => Effect {
                reads: mentioned | RSP,
                writes: RSP,
            },
            LineKind::Pop { reg } => {
                let w = if reg != REG_NONE && reg <= REG_GP_MAX {
                    1u16 << reg
                } else {
                    0
                };
                Effect {
                    reads: RSP,
                    writes: w | RSP,
                }
            }
            _ => {
                if is_string_instruction(t) {
                    // Implicit operands only; `rep` variants also read and
                    // update the count.  Treating the data register as both
                    // read and written is conservative for `lods`/`stos`.
                    const STRING_REGS: u16 = RAX | RCX | (1 << 6) | (1 << 7);
                    return Effect {
                        reads: STRING_REGS | mentioned,
                        writes: STRING_REGS,
                    };
                }
                if !mnemonic_is_known(t) {
                    // Unknown: keep every mentioned register live.
                    return Effect {
                        reads: mentioned,
                        writes: 0,
                    };
                }
                let mut reads = mentioned;
                let mut writes = 0u16;
                let dest = get_dest_reg(info);
                if dest != REG_NONE && dest <= REG_GP_MAX {
                    let bit = 1u16 << dest;
                    writes |= bit;
                    // A pure write does not read its destination; anything else
                    // (add, cmov, inc, shifts, xchg) does.
                    if is_pure_write_mnemonic(t) {
                        // Shared source-mentions-dest guard (covers the NDD
                        // middle operand, 16-bit and high-byte reads, and
                        // strips `#` comments so comment commas never move
                        // the source boundary).
                        let src_reads_dest = src_mentions_family(t, dest);
                        let name64 = REG_NAMES[0][dest as usize];
                        let name32 = REG_NAMES[1][dest as usize];
                        // A partial write (8/16-bit destination) preserves the
                        // rest of the register, so the old value stays live.
                        let dst_text = t[t.rfind(',').map(|c| c + 1).unwrap_or(0)..].trim_ascii();
                        let full = dst_text == name64 || dst_text == name32;
                        if !src_reads_dest && full {
                            reads &= !bit;
                        }
                    } else if t.starts_with("xor") {
                        // `xor{l,q} %rX, %rX` — a self-xor is a ZERO: the
                        // result is independent of the old value, so a
                        // full-width one reads nothing (the partial w/b
                        // forms still preserve the upper bits and stay
                        // conservative). The plain pure-write path cannot
                        // express this: its source mentions the destination
                        // by construction. This is the idiomatic %al=0
                        // before variadic calls and every `xorl %eax,%eax`
                        // zeroing — modelling them as reads pinned the
                        // accumulator live across whole regions and blocked
                        // every copy fold feeding a later zeroing.
                        let ops = t.split_once(' ').and_then(|(_, rest)| {
                            rest.split_once(',')
                                .map(|(a, b)| (a.trim_ascii(), b.trim_ascii()))
                        });
                        if let Some((a, b)) = ops {
                            if a == b {
                                let name64 = REG_NAMES[0][dest as usize];
                                let name32 = REG_NAMES[1][dest as usize];
                                if a == name64 || a == name32 {
                                    reads &= !bit;
                                }
                            }
                        }
                    }
                }
                // Implicit operands — one source of truth (types.rs oracle).
                // The old hand arms missed `cltd` entirely (its RAX read is
                // load-bearing: deleting a dividend definition because
                // liveness could not see the consumption was a live bug),
                // taxed every SSE `divsd`/`mulsd` through a starts_with
                // ("div") prefix match, claimed a phantom `cqto` write of
                // %rax, and knew nothing of syscall/string/loop kills.
                //
                // `mentioned` is the UNION oracle (reads ∪ writes), which is
                // the right "does this line touch F" answer for the folding
                // passes but the WRONG read set here: an implicit write is
                // not an observation, and reading every written register
                // would cancel every kill this module exists to compute
                // (`syscall` would never retire %r11, `cltd` never %rdx).
                // The exact read set is: explicit mentions, minus the
                // implicit write half, plus the implicit read half.
                //
                // Width exactness: the oracle's write half is the UNION of
                // any-width writes, but this analysis works on 64-bit
                // families, where only a ≥32-bit write is a KILL.  A partial
                // implicit write (`lahf` → AH, `xlat`/`divb`/`lodsb` → AX,
                // `fnstsw` → AX, the conditional `cmpxchg`/`xbegin`
                // accumulator reloads) leaves the rest of the old family
                // observable, so it must keep the family LIVE (model it as
                // a read), never kill it: killing on `lahf` deleted a
                // `movslq %eax, %rax` whose high bits `ret` still returned.
                let (ir, iw, iwf) =
                    crate::backend::x86::codegen::peephole::types::implicit_r_w_full(t.as_bytes());
                reads &= !iwf; // only a full implicit write cancels an observation
                reads |= ir;
                reads |= iw & !iwf; // partial implicit writes preserve the old value
                writes |= iwf; // only a full implicit write is a kill
                if t.starts_with("xchg") || t.starts_with("cmpxchg") || t.starts_with("xadd") {
                    reads |= mentioned;
                    writes |= mentioned;
                }
                Effect { reads, writes }
            }
        }
    }
}

/// `upper32_zero_transfer` through an exact cross-solve cache.  The transfer
/// is a pure function of `(LineInfo, text)`, and the solves one function
/// sees between rewrites differ in a handful of lines, so re-deriving every
/// line's operand model per solve was most of the solver's cost.  A hit is
/// verified against the full key (info and text), so a collision or a
/// same-text line in another state only costs a recomputation, never a
/// wrong answer.
fn upper32_zero_transfer_cached(info: &LineInfo, t: &str) -> (u16, u16) {
    use std::hash::{Hash, Hasher};
    type Cache = std::collections::HashMap<
        u64,
        (LineInfo, Box<str>, (u16, u16)),
        std::hash::BuildHasherDefault<crate::common::fx_hash::FxHasher>,
    >;
    // Bounded: a whole-cache reset is O(1) amortised and the working set
    // of one translation unit's hot function is far below the cap.
    const CAP: usize = 1 << 15;
    thread_local! {
        static CACHE: std::cell::RefCell<Cache> = std::cell::RefCell::new(Cache::default());
    }
    // Keyed on the text alone: the full-key check below keeps it exact, and
    // a same-text line in another state (pinned, nop) merely evicts.  Not
    // hashing `LineInfo` here keeps the key one short `FxHasher` pass over
    // bytes the caller already holds; the `i == info` guard on a hit is a
    // handful of integer compares.
    let mut h = crate::common::fx_hash::FxHasher::default();
    t.hash(&mut h);
    let key = h.finish();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((i, s, r)) = c.get(&key)
            && i == info
            && **s == *t
        {
            return *r;
        }
        let r = upper32_zero_transfer(info, t);
        if c.len() >= CAP {
            c.clear();
        }
        c.insert(key, (*info, t.into(), r));
        r
    })
}

/// The GP families `writes_family` answers "may write" for, as one mask:
/// its definition in closed form (architectural implicit writes, plus the
/// classified destination unless the line only implicitly READS it), so a
/// line's operand model is derived once instead of once per family.  The
/// inline-asm arm (every family) is the caller's kill-all.
fn may_write_families(info: &LineInfo, t: &str) -> u16 {
    let bytes = t.as_bytes();
    let mut written = implicit_write_refs(bytes);
    let dest = super::helpers::get_dest_reg(info);
    if dest <= REG_GP_MAX && implicit_read_refs(bytes) & (1u16 << dest) == 0 {
        written |= 1u16 << dest;
    }
    written
}

/// Exact, cheap necessary condition for `s_write_proves_upper_zero`: that
/// predicate proves only through a dword destination naming `fam` (so the
/// line ends in that name after a separator) or one of the 64-bit idioms
/// `xorq`/`subq` self, `movq $imm`, `movabs[q] $imm`.  Skips its operand
/// parse on the common 64-bit and memory-destination writes.
fn write_may_prove_upper_zero(t: &str, fam: RegId) -> bool {
    let t = t.trim_ascii_end();
    let dword = REG_NAMES[1][fam as usize];
    if let Some(head) = t.strip_suffix(dword)
        && head.ends_with([',', ' ', '\t'])
    {
        return true;
    }
    matches!(
        t.split_whitespace().next(),
        Some("xorq" | "subq" | "movq" | "movabs" | "movabsq")
    )
}

/// `(keep, proved)` for the upper-half-zero dataflow at one line:
/// `out = (in & keep) | proved`.  Mirrors one backward step of
/// `relay_and_lea::upper32_zero_at` (see `upper32_zero_into_label`).
fn upper32_zero_transfer(info: &LineInfo, t: &str) -> (u16, u16) {
    const KILL_ALL: (u16, u16) = (0, 0);
    if info.is_nop() {
        return (ALL, 0);
    }
    if info.pinned || info.kind == LineKind::InlineAsm {
        return KILL_ALL;
    }
    if info.is_barrier() {
        // Labels and direct jumps write no register: the CFG carries the
        // fact across them.  Every other barrier (call, push/pop, ret,
        // non-inert directive, indirect jump) is a uniform stop.
        return if matches!(
            info.kind,
            LineKind::Label | LineKind::Jmp | LineKind::CondJmp
        ) {
            (ALL, 0)
        } else {
            KILL_ALL
        };
    }
    if t.bytes().any(|b| b.is_ascii_uppercase()) {
        return KILL_ALL;
    }
    let t = t.strip_prefix("lock ").unwrap_or(t);
    if super::helpers::has_implicit_reg_usage(t) {
        return KILL_ALL;
    }
    let written = may_write_families(info, t);
    let (mut keep, mut proved) = (ALL, 0u16);
    for fam in 0..=REG_GP_MAX {
        let bit = 1u16 << fam;
        if written & bit == 0 {
            continue; // not written: the fact flows through unchanged
        }
        keep &= !bit;
        if write_may_prove_upper_zero(t, fam)
            && super::relay_and_lea::s_write_proves_upper_zero(t, fam)
        {
            proved |= bit;
        }
    }
    (keep, proved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    /// `live_after_dropping_region` probes in place and restores the saved
    /// span.  Two contracts, both against independent oracles:
    /// 1. the answer equals the old clone-the-world implementation's
    ///    (clone + `reanalyse_with` on the clone);
    /// 2. after the call, EVERY caller-visible field of the probed
    ///    `FileLiveness` is bit-identical to its pre-call state.
    #[test]
    fn dropping_region_probe_restores_state_exactly() {
        // A loop so the neutralised question is non-trivial (the carriers
        // look live across the backedge) plus a second function to prove
        // out-of-span state is untouched.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    # LCCC_RET_RAX 1\n",
            "    movl $1, %ecx\n",
            ".L1:\n",
            "    setne %r8b\n",
            "    movzbl %r8b, %r8d\n",
            "    addl %ecx, %eax\n",
            "    subl $1, %edi\n",
            "    jne .L1\n",
            "    ret\n.cfi_endproc\n",
            "g:\n.cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    # LCCC_RET_RAX 1\n",
            "    movl $2, %eax\n",
            "    ret\n.cfi_endproc\n",
        );
        let store = LineStore::new(asm.to_string());
        let infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let line = |text: &str| {
            (0..store.len())
                .find(|&i| store.get(i).trim_ascii() == text)
                .unwrap()
        };
        let mut lv = FileLiveness::new(&store, &infos);
        let setcc = line("setne %r8b");
        let widen = line("movzbl %r8b, %r8d");
        let mask = 1u16 << 8; // %r8 family

        // Oracle 1: the old implementation, verbatim.
        let mut oracle = FileLiveness {
            live_out: lv.live_out.clone(),
            live_in: lv.live_in.clone(),
            known: lv.known.clone(),
            structural: lv.structural.clone(),
            effect: lv.effect.clone(),
            func_of: lv.func_of.clone(),
            textual_ret: lv.textual_ret.clone(),
        };
        let (start, end) = oracle.recorded_function(&store, &infos, widen).unwrap();
        oracle.reanalyse_with(&store, &infos, start, end, Some((setcc, widen, mask)));
        let expected = oracle.known[widen].then(|| oracle.live_out[widen]);

        // Snapshot every caller-visible field before the in-place probe.
        let snap = (
            lv.live_out.clone(),
            lv.live_in.clone(),
            lv.known.clone(),
            lv.structural.clone(),
            lv.effect.clone(),
            lv.func_of.clone(),
            lv.textual_ret.clone(),
        );
        let got = lv.live_after_dropping_region(&store, &infos, widen, setcc, widen, mask);
        assert_eq!(got, expected, "probe answer matches the clone oracle");
        assert_eq!(snap.0, lv.live_out, "live_out restored");
        assert_eq!(snap.1, lv.live_in, "live_in restored");
        assert_eq!(snap.2, lv.known, "known restored");
        assert_eq!(snap.3, lv.structural, "structural restored");
        assert_eq!(snap.4, lv.effect, "effect restored");
        assert_eq!(snap.5, lv.func_of, "func_of untouched");
        assert_eq!(snap.6, lv.textual_ret, "textual_ret restored");

        // A second probe on the same span must agree with the first (the
        // restore left no residue that could steer a later answer).
        let again = lv.live_after_dropping_region(&store, &infos, widen, setcc, widen, mask);
        assert_eq!(got, again, "probe is idempotent");
    }

    /// `DeadWriteRefresh` keeps the solution exact without re-solving per
    /// deletion: after each deletion every family outside the pending read
    /// set answers exactly what a fresh solve of the edited text answers,
    /// on every line; a query on a pending family re-solves first, after
    /// which all families agree.  The fixture is a loop whose dead writes
    /// cascade (deleting `movl %edi, %r8d` frees nothing; deleting
    /// `movl %r9d, %r10d` makes `%r9d`'s producer dead in turn).
    #[test]
    fn dead_write_refresh_stays_exact() {
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    # LCCC_RET_RAX 1\n",
            "    movl $1, %ecx\n",
            "    movl %esi, %r9d\n",
            ".L1:\n",
            "    movl %edi, %r8d\n",
            "    movl %r9d, %r10d\n",
            "    addl %ecx, %eax\n",
            "    subl $1, %edi\n",
            "    jne .L1\n",
            "    ret\n.cfi_endproc\n",
        );
        let store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let line = |text: &str| {
            (0..store.len())
                .find(|&i| store.get(i).trim_ascii() == text)
                .unwrap()
        };
        let mut lv = FileLiveness::new(&store, &infos);
        let mut upkeep = DeadWriteRefresh::new();
        let agree = |lv: &FileLiveness, infos: &[LineInfo], families: u16, what: &str| {
            let fresh = FileLiveness::new(&store, infos);
            for j in 0..store.len() {
                if infos[j].is_nop() {
                    continue;
                }
                for fam in 0..=REG_GP_MAX {
                    if families & (1u16 << fam) != 0 {
                        assert_eq!(
                            lv.live_after(j, fam),
                            fresh.live_after(j, fam),
                            "{what}: line {j} {:?} family {fam}",
                            store.get(j)
                        );
                    }
                }
            }
        };
        // Deletion order of a forward scan; each line is dead when deleted.
        for text in ["movl %edi, %r8d", "movl %r9d, %r10d"] {
            let i = line(text);
            assert_eq!(
                lv.live_after(i, register_family_fast(text.rsplit(' ').next().unwrap())),
                Some(false)
            );
            let reads = lv
                .deletion_changes_only_reads(i)
                .expect("deferrable deletion");
            upkeep.delete(&mut lv, &store, &mut infos, i);
            let pending = upkeep.stale.map_or(0, |(_, f)| f);
            assert_eq!(pending & reads, reads, "{text}: its reads are pending");
            agree(&lv, &infos, !pending, &format!("after deleting {text}"));
        }
        // `%r9d`'s producer is dead only in the re-solved program: asking
        // about family 9 must refresh, then everything agrees.
        let producer = line("movl %esi, %r9d");
        upkeep.before_query(&mut lv, &store, &infos, producer, 9);
        assert!(
            upkeep.stale.is_none(),
            "a pending family forces the re-solve"
        );
        assert_eq!(lv.live_after(producer, 9), Some(false));
        agree(&lv, &infos, u16::MAX, "after the forced re-solve");
    }

    /// Drift alarm for the dest-only contract.  Since S03 the liveness
    /// classifier DELEGATES to `helpers::is_read_modify_write` (the
    /// sha256 regression was exactly such a drift: rorx present in one
    /// list, missing in the other).  This table pins the semantic result
    /// itself, so any future edit of the shared predicate that moves one
    /// of these mnemonics fails loudly here instead of silently killing
    /// liveness-driven peepholes on real workloads.
    ///
    /// SCOPE: these rows pin the SHARED PREDICATE. Liveness additionally
    /// gates on `mnemonic_is_known` (unknown mnemonics take the
    /// reads-everything/writes-nothing path without consulting the
    /// predicate), so the bzhi/bextr/pext/pdep rows below are LATENT pins:
    /// they pass today, and they go live the day those mnemonics join
    /// KNOWN (the backend emits none of them yet).
    #[test]
    fn pure_write_table_matches_the_shared_rmw_predicate() {
        // Dest-only: written, never read.
        let dest_only = [
            "movq %rax, %rbx",
            "movl $1, %eax",
            "movabsq $4611686018427387904, %rax",
            "movzbl %al, %eax",
            "movsbl %al, %eax",
            "movslq %eax, %rbx",
            "leaq 8(%rcx), %rax",
            "sete %al",
            "lzcntq %rbx, %rax",
            "tzcntl %ebx, %eax",
            "popcntq %rbx, %rax",
            "shlxl %ebx, %ecx, %edx",
            "shrxq %rbx, %rcx, %rdx",
            "sarxl %ebx, %ecx, %edx",
            "rorxl $7, %edx, %eax",
            "andnl %ebx, %ecx, %edx",
            "bzhil %ebx, %ecx, %edx",
            "bextrl %ebx, %ecx, %edx",
            "pextq %rbx, %rcx, %rdx",
            "pdepq %rbx, %rcx, %rdx",
            "blsrl %ebx, %eax",
            "blsil %ebx, %eax",
            "blsmskl %ebx, %eax",
            // Vector/FP -> GPR transfers (whole GPR written, r32 forms
            // zero-extend).
            "vmovq %xmm0, %rax",
            "vmovd %xmm0, %eax",
            "movd %xmm1, %ecx",
            "vpextrd $1, %xmm0, %eax",
            "pextrw $3, %xmm2, %edx",
            "vpmovmskb %ymm0, %eax",
            "vmovmskpd %xmm0, %eax",
            "movmskps %xmm3, %r8d",
            "cvttsd2si %xmm0, %rax",
            "vcvttss2si %xmm1, %eax",
            "vcvtsd2si %xmm2, %rcx",
        ];
        for line in dest_only {
            assert!(
                is_pure_write_mnemonic(line) && !is_read_modify_write(line),
                "{line}: must be dest-only in BOTH models"
            );
        }
        // Non-destructive operand forms of otherwise-RMW mnemonics: the
        // three-operand `imul` (magic-division staple) and the APX NDD
        // forms write their destination without reading it.
        let ndd_forms = [
            "imull $5, %ebx, %r9d",
            "imulq %rbx, %rcx, %rdx",
            "imull (%rbx), %ecx, %eax",
            "addl $1, %eax, %ebx",
            "subq %rax, %rbx, %r12",
            "negl %eax, %ebx",
            "notq %rax, %r11",
        ];
        for line in ndd_forms {
            assert!(
                is_pure_write_mnemonic(line) && !is_read_modify_write(line),
                "{line}: extra-destination form must be dest-only in BOTH models"
            );
        }
        // Read-modify-write (or conservatively modelled as such).
        let rmw = [
            "addq %rax, %rbx",
            "imull %ebx, %r9d",     // two-operand imul reads its destination
            "imulq %rbx",           // one-operand: implicit %rax/%rdx world
            "addl $1, %eax",        // two-operand ALU reads the destination
            "negl %eax",            // one-operand unary reads the destination
            "shldl $2, %esi, %edi", // bits shift THROUGH the destination
            "addsd %xmm0, %xmm1",   // SSE look-alike stays RMW (exact match)
            "cmovneq %rax, %rbx",   // dest kept when the condition is false
            "bsfq %rbx, %rax",      // dest untouched for a zero source
            "bsrq %rbx, %rax",
            "mulxl %ebx, %ecx, %edx", // implicit %rdx read, two dests
            "rolq $1, %rax",          // two-operand rotates read the dest
            "shrq $1, %rax",
            "incq %rax",
            "xchgl %eax, %ebx",
            "adoxq %rbx, %rax",
            // LEA with the dest inside its own address reads the dest;
            // the unified model reports RMW and the classifier keeps the
            // old value live (soundness over the rare self-address form).
            "leaq 4(%rax), %rax",
        ];
        for line in rmw {
            assert!(
                !is_pure_write_mnemonic(line) && is_read_modify_write(line),
                "{line}: must stay read-modify-write in BOTH models"
            );
        }
    }

    /// zlib_ng_adler32: a vector-lane extract into %rax after a loop must
    /// KILL %rax, not read it. As a phantom read it kept the fill loop's
    /// dead `movq %rdx, %rax` (and the in-place `movzbl` feeding it) alive
    /// for 2M iterations: liveness flowed from the extract back over the
    /// loop's exit edge into the loop body.
    #[test]
    fn vector_extract_into_gpr_kills_it_across_a_loop_exit() {
        let (store, _infos, lv) = build(concat!(
            ".text\nf:\n.cfi_startproc\n",
            ".L1:\n",
            "    movq %rdx, %rax\n",
            "    movb %dl, (%rcx, %rsi)\n",
            "    addq $1, %rsi\n",
            "    cmpq $8, %rsi\n",
            "jb .L1\n",
            "    vmovq %xmm0, %rax\n",
            "    movq %rax, 8(%rsp)\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        let copy = (0..store.len())
            .find(|&n| store.get(n).trim_ascii() == "movq %rdx, %rax")
            .expect("fixture has the copy");
        assert_eq!(lv.live_after(copy, 0), Some(false));
    }

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>, FileLiveness) {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FileLiveness::new(&store, &infos);
        (store, infos, lv)
    }

    /// Every line's (known, live-in, live-out, effect) equals a from-scratch
    /// analysis of the edited text.
    fn assert_same_as_fresh(store: &LineStore, infos: &[LineInfo], lv: &FileLiveness) {
        let full = FileLiveness::new(store, infos);
        for n in 0..store.len() {
            let got = (
                lv.known[n],
                lv.known[n].then(|| (lv.live_in[n], lv.live_out[n], lv.effect[n])),
            );
            let want = (
                full.known[n],
                full.known[n].then(|| (full.live_in[n], full.live_out[n], full.effect[n])),
            );
            assert_eq!(got, want, "line {n} `{}`", store.get(n));
        }
    }

    fn edit(store: &mut LineStore, infos: &mut [LineInfo], n: usize, text: &str) {
        replace_line(store, &mut infos[n], n, format!("    {text}"));
    }

    /// Straight-line fold (the copy-propagation shape): spliced, exact.
    /// A frameless function that allocates `%rbp` as a GPR: its `%rsp`-slot
    /// spills and reloads must not read `%rbp`. The slot classification used
    /// to hard-code the rbp bit for both slot bases, so the spill below kept
    /// the byte value live past its only consumer (and, in
    /// `zlib_ng_adler32_combine`, across two whole loops).
    #[test]
    fn rsp_slot_traffic_is_not_a_read_of_an_allocated_rbp() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    subq $16, %rsp\n",
            "    movl %edi, %ebp\n",
            "    movb %bpl, (%rsi)\n",
            "    movq %rax, 8(%rsp)\n",
            "    movl $1, %ebp\n",
            "    movq 8(%rsp), %rax\n",
            "    addq %rbp, %rax\n",
            "    addq $16, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let consumer = line_of(&store, "movb %bpl, (%rsi)");
        assert_eq!(
            lv.live_after(consumer, 5),
            Some(false),
            "the spill does not read %rbp"
        );
        // The second value really is read across the reload: still live.
        let reload = line_of(&store, "movq 8(%rsp), %rax");
        assert_eq!(
            lv.live_after(reload, 5),
            Some(true),
            "addq still reads %rbp"
        );
    }

    /// `-mfunction-return=thunk-extern`: the function returns by jumping to
    /// `__x86_return_thunk`.  The thunk is the return, so every line of such
    /// a function has the liveness of its `ret` twin; read as a jump to an
    /// unknown symbol it made the whole function unanalysable, and every
    /// kernel function with it.
    #[test]
    fn a_return_through_the_thunk_is_a_return() {
        let body = |ret: &str| {
            format!(
                concat!(
                    "f:\n",
                    ".cfi_startproc\n",
                    "# LCCC_RET_RDX 0\n",
                    "# LCCC_RET_RAX 1\n",
                    "    pushq %rbx\n",
                    "    movq %rdi, %rbx\n",
                    "    testq %rsi, %rsi\n",
                    "    je .Lzero\n",
                    "    movq 48(%rsp), %rax\n",
                    "    leaq 152(%rax), %rax\n",
                    "    movq %rax, %rdx\n",
                    "    movq (%rdx), %r11\n",
                    "    leaq (%r11, %rbx), %rax\n",
                    "    popq %rbx\n",
                    "    {ret}\n",
                    ".Lzero:\n",
                    "    xorl %eax, %eax\n",
                    "    popq %rbx\n",
                    "    {ret}\n",
                    ".cfi_endproc\n",
                ),
                ret = ret
            )
        };
        let (plain_store, plain_infos, plain) = build(&body("ret"));
        let (thunk_store, thunk_infos, thunk) = build(&body("jmp __x86_return_thunk"));
        assert_eq!(plain_store.len(), thunk_store.len());
        let mut returns = 0;
        for n in 0..plain_store.len() {
            assert_eq!(
                plain_infos[n].kind,
                thunk_infos[n].kind,
                "line {n} `{}`",
                thunk_store.get(n)
            );
            returns += usize::from(thunk_infos[n].kind == LineKind::Ret);
            assert!(
                thunk.known[n] == plain.known[n],
                "line {n} `{}`: analysed with ret, {} with the thunk",
                thunk_store.get(n),
                if thunk.known[n] { "also" } else { "not" }
            );
            for fam in 0..16u8 {
                assert_eq!(
                    thunk.live_after(n, fam),
                    plain.live_after(n, fam),
                    "family {fam} after line {n} `{}`",
                    thunk_store.get(n)
                );
            }
        }
        assert_eq!(returns, 2);
        // What the answer is good for: behind the load the two temporaries
        // are provably dead, although a branch and a return follow.
        let load = line_of(&thunk_store, "movq (%rdx), %r11");
        assert_eq!(thunk.live_after(load, 2), Some(false), "%rdx");
        assert_eq!(
            thunk.live_after(load, 3),
            Some(true),
            "%rbx is read by the lea"
        );
        let last = line_of(&thunk_store, "xorl %eax, %eax");
        assert_eq!(thunk.live_after(last, 0), Some(true), "%rax is returned");
        assert_eq!(thunk.live_after(last, 2), Some(false), "LCCC_RET_RDX 0");
    }

    /// A function around one inline-asm region.  `%r8`..`%r11`, `%rcx` and
    /// `%rdx` are written in front of it and never named behind it, `%rsi` is
    /// read only in the block `.Lcold` and `%rdi` only behind the region: what
    /// is live in front of the region is what it reads plus what its
    /// successors do.
    fn live_before_asm(region: &str) -> Option<u16> {
        let asm = format!(
            concat!(
                "f:\n",
                ".cfi_startproc\n",
                "# LCCC_RET_RDX 0\n",
                "# LCCC_RET_RAX 1\n",
                "    movq (%rdi), %r8\n",
                "    movq 8(%rdi), %r9\n",
                "    movq 16(%rdi), %r10\n",
                "    movq 24(%rdi), %r11\n",
                "    xorl %ecx, %ecx\n",
                "    xorl %edx, %edx\n",
                "#APP\n",
                "{region}",
                "#NO_APP\n",
                "    movq %rdi, %rax\n",
                "    ret\n",
                ".Lcold:\n",
                "    movq %rsi, %rax\n",
                "    ret\n",
                ".cfi_endproc\n",
            ),
            region = region
        );
        // As the peephole sees it: the region's lines pinned as inline asm.
        let store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        let lv = FileLiveness::new(&store, &infos);
        let marker = line_of(&store, "xorl %edx, %edx");
        let mut live = 0u16;
        for fam in 0..16u8 {
            match lv.live_after(marker, fam) {
                Some(true) => live |= 1 << fam,
                Some(false) => {}
                None => return None,
            }
        }
        Some(live)
    }

    const RSI_BIT: u16 = 1 << 6;
    const RDI_BIT: u16 = 1 << 7;
    const R8_BIT: u16 = 1 << 8;
    const R9_BIT: u16 = 1 << 9;
    const R10_BIT: u16 = 1 << 10;
    const R11_BIT: u16 = 1 << 11;
    /// Live behind the region whatever it does: the argument the fall-through
    /// returns, and what a `ret` owes the caller.
    const FALLS_THROUGH: u16 =
        RDI_BIT | RBX | RBP | RSP | (1 << 12) | (1 << 13) | (1 << 14) | (1 << 15);

    /// A static key is a `jmp` objtool turns into a NOP plus a table entry
    /// (or the NOP as bytes): it reads no register, and it may leave for its
    /// label.  Before, it read all sixteen and nothing in front of it died.
    #[test]
    fn a_static_key_reads_nothing_and_may_take_its_label() {
        for key in [
            concat!(
                "    1: jmp .Lcold # objtool NOPs this\n",
                "    .pushsection __jump_table,  \"aw\"\n",
                "    .balign 8\n",
                "    .long 1b - .\n",
                "    .long .Lcold - .\n",
                "    .quad key + 2 - .\n",
                "    .popsection\n",
            ),
            concat!(
                "    1:.byte 0x0f,0x1f,0x44,0x00,0x00\n",
                "    .pushsection __jump_table,  \"aw\"\n",
                "    .balign 8\n",
                "    .long 1b - .\n",
                "    .long .Lcold - .\n",
                "    .quad key + 0 - .\n",
                "    .popsection\n",
            ),
        ] {
            assert_eq!(live_before_asm(key), Some(FALLS_THROUGH | RSI_BIT), "{key}");
        }
        // Without the table entry nothing names the label: no edge, and
        // %rsi, read only there, is dead.
        assert_eq!(live_before_asm("    nop\n"), Some(FALLS_THROUGH));
    }

    /// An asm reads the registers its text names and the ones its mnemonics
    /// imply; it kills none.
    #[test]
    fn an_asm_region_reads_what_it_names() {
        for (region, reads) in [
            ("    lock orb $2,(%rcx)\n", RCX),
            ("    addl $1, %gs:__preempt_count(%rip)\n", 0),
            ("    btq %rcx,(%rdx)\n", RCX | RDX),
            ("    movb (%rdx),%cl\n", RCX | RDX),
            ("    xchgl %ecx, (%rdx)\n", RCX | RDX),
            ("    lock cmpxchgq %r8, (%r9)\n", RAX | R8_BIT | R9_BIT),
            ("    1:\tmovq %r10, %r11\n", R10_BIT | R11_BIT),
            ("    rep; nop\n", RCX),
            ("    lock; incl (%r8)\n", R8_BIT),
            (
                "    sete %cl\n    cmovneq %r9, %r10\n",
                RCX | R9_BIT | R10_BIT,
            ),
            // Code in a pushed executable section is patched in here.
            (
                concat!(
                    "    nop\n",
                    "    .pushsection .altinstr_replacement, \"ax\"\n",
                    "    movq %r9, %r11\n",
                    "    .popsection\n",
                ),
                R9_BIT | R11_BIT,
            ),
            // A jump to a numeric label the region defines stays inside it.
            ("    1: decq %r8\n    jne 1b\n", R8_BIT),
        ] {
            assert_eq!(
                live_before_asm(region),
                Some(FALLS_THROUGH | reads),
                "{region}"
            );
        }
        // An exception fixup is an edge to its label.
        assert_eq!(
            live_before_asm(concat!(
                "    1:\tmovb (%rdx),%cl\n",
                "    .pushsection __ex_table,\"a\"\n",
                "    .long 1b - ., .Lcold - .\n",
                "    .popsection\n",
            )),
            Some(FALLS_THROUGH | RSI_BIT | RCX | RDX)
        );
    }

    /// Whatever the scanner does not understand keeps the old answer: the
    /// region reads every register.
    #[test]
    fn an_asm_region_that_is_not_understood_reads_everything() {
        for region in [
            "    call helper\n",
            "    syscall\n",
            "    1: ud2\n",
            "    int3\n",
            "    ret\n",
            "    jmp elsewhere\n",
            "    jmp *%r8\n",
            "    jrcxz .Lcold\n",
            "    jne 7f\n",
            "    vmcall\n",
            "    fxsave (%r8)\n",
            "    shldq %r8, %r9\n",
            "    extable_type_reg reg=%r8, type=3\n",
            "    .intel_syntax noprefix\n    mov rax, rbx\n",
            "    .byte 0x0f, 0xa2\n",
            "    .long 0x90909090\n",
            "    .pushsection .altinstr_replacement, \"ax\"\n    .byte 0x0f,0x01,0xcb\n    .popsection\n",
            "    .pushsection unknown_flags\n    .long 1\n    .popsection\n",
            "    .pushsection __ex_table,\"a\"\n    .long 1\n",
            "    .popsection\n",
            "    nop; .ascii \"a;b\"\n",
            "    MOVQ %r8, %r9\n",
        ] {
            assert_eq!(live_before_asm(region), Some(u16::MAX), "{region}");
        }
    }

    /// The pieces the region scanner is made of.
    #[test]
    fn asm_region_scanner_parts() {
        for nop in [
            "0x90",
            "0x66,0x90",
            "0x0f,0x1f,0x44,0x00,0x00",
            " 0x0f, 0x1f, 0x44, 0x00, 0x00 ",
            "15,31,68,0,0",
            "0x0f,0x1f,0x84,0x00,0x00,0x00,0x00,0x00",
        ] {
            assert!(asm_bytes_are_a_nop(nop), "{nop}");
        }
        for other in [
            "",
            "0x0f,0x0b",
            "0x0f,0x1f,0x44,0x00",
            "0x0f,0x1f,0x44,0x00,0x00,0x90",
            "0x190",
            "sym",
            "0x0f,0x1f,0x84,0,0,0,0,0,0",
        ] {
            assert!(!asm_bytes_are_a_nop(other), "{other}");
        }
        assert!(!asm_section_may_execute("__jump_table,  \"aw\""));
        assert!(!asm_section_may_execute(
            ".discard.annotate_insn,\"M\",@progbits,8"
        ));
        assert!(asm_section_may_execute(".altinstr_replacement, \"ax\""));
        assert!(asm_section_may_execute(".text"));
        assert!(asm_section_may_execute("mystery, @progbits"));
        for known in [
            "movq", "movzbl", "leaq", "orb", "btsq", "xaddl", "cmpxchgq", "sete", "setnae",
            "cmovne", "cmovnel", "cmovl", "cmovll", "nopw", "pushq", "popfq", "cqto", "rdmsr",
        ] {
            assert!(asm_mnemonic_is_modelled(known), "{known}");
        }
        for unknown in [
            "call",
            "jmp",
            "ret",
            "shldq",
            "inb",
            "outb",
            "movsd",
            "addsd",
            "fxsave",
            "leave",
            "cmpxchg16b",
            "setx",
            "cmovx",
            "cmov",
            "set",
            "loop",
            "syscall",
            "hlt",
        ] {
            assert!(!asm_mnemonic_is_modelled(unknown), "{unknown}");
        }
        assert!(asm_mnemonic_is_jump("jmp") && asm_mnemonic_is_jump("jnae"));
        assert!(!asm_mnemonic_is_jump("jrcxz") && !asm_mnemonic_is_jump("j"));
    }

    /// Only the plain thunk is a return.  The i686 callee-pops thunk reads its
    /// return address from %ecx, a conditional jump there is no return any
    /// kernel patches, and another symbol is another function.
    #[test]
    fn only_the_plain_thunk_jump_is_classified_as_a_return() {
        assert_eq!(
            classify_line("    jmp __x86_return_thunk").kind,
            LineKind::Ret
        );
        assert_eq!(
            classify_line("\tjmp __x86_return_thunk  # return").kind,
            LineKind::Ret
        );
        for other in [
            "    jmp __x86_return_thunk_ecx",
            "    jmp __x86_return_thunk@PLT",
            "    jmp __x86_return_thunkx",
            "    jmp x__x86_return_thunk",
            "    jmp .L__x86_return_thunk",
        ] {
            assert_eq!(classify_line(other).kind, LineKind::Jmp, "{other}");
        }
        assert_eq!(
            classify_line("    jne __x86_return_thunk").kind,
            LineKind::CondJmp
        );
        assert_eq!(
            classify_line("    call __x86_return_thunk").kind,
            LineKind::Call
        );
    }

    #[test]
    fn refresh_span_splices_a_straight_line_fold() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "# LCCC_RET_RDX 0\n",
            "    movq %rdi, %rcx\n",
            "    movq %rcx, %rax\n",
            "    addq %rsi, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos, mut lv) = build(asm);
        let lo = line_of(&store, "movq %rdi, %rcx");
        let hi = line_of(&store, "movq %rcx, %rax");
        mark_nop(&mut infos[lo]);
        edit(&mut store, &mut infos, hi, "movq %rdi, %rax");
        assert!(
            lv.try_refresh_span(&store, &infos, lo, hi),
            "local edit must splice"
        );
        assert_same_as_fresh(&store, &infos, &lv);
    }

    /// Deleting a dead copy inside a loop whose source the next line reads:
    /// the lost read is covered by the code after the span, so it splices.
    #[test]
    fn refresh_span_splices_a_nop_in_a_loop() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "# LCCC_RET_RDX 0\n",
            "    movq $0, %rax\n",
            ".Lloop:\n",
            "    movq %rax, %r9\n",
            "    addq %r8, %rax\n",
            "    subq $1, %rcx\n",
            "    jne .Lloop\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, mut infos, mut lv) = build(asm);
        let n = line_of(&store, "movq %rax, %r9");
        mark_nop(&mut infos[n]);
        assert!(lv.try_refresh_span(&store, &infos, n, n));
        assert_same_as_fresh(&store, &infos, &lv);
    }

    /// The loop-carried case the entry-equality test alone gets wrong:
    /// `%r8` is read only by the edited line, so after the edit it is dead
    /// in the whole loop -- but it passes straight through the span from
    /// the stale anchor, so the entry live-in looks unchanged.  The splice
    /// must refuse, and `refresh_span` must still produce the exact result.
    #[test]
    fn refresh_span_refuses_a_loop_sustained_register() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "# LCCC_RET_RDX 0\n",
            "    movq $0, %rax\n",
            ".Lloop:\n",
            "    addq %r8, %rax\n",
            "    subq $1, %rcx\n",
            "    jne .Lloop\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos, mut lv) = build(asm);
        let n = line_of(&store, "addq %r8, %rax");
        assert_eq!(
            lv.live_after(n, 8),
            Some(true),
            "loop-carried before the edit"
        );
        edit(&mut store, &mut infos, n, "addq $1, %rax");
        assert!(
            !lv.try_refresh_span(&store, &infos, n, n),
            "must not splice"
        );
        lv.refresh_span(&store, &infos, n, n);
        assert_same_as_fresh(&store, &infos, &lv);
        assert_eq!(lv.live_after(n, 8), Some(false));
    }

    /// An edit that changes the span's live-in (a new upward-exposed read)
    /// changes predecessors' solutions: refused, then recomputed exactly.
    /// So is any span touching a structural line.
    #[test]
    fn refresh_span_refuses_entry_changes_and_structural_lines() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "# LCCC_RET_RDX 0\n",
            "    movq $1, %rax\n",
            "    movq $2, %rdx\n",
            ".Lnext:\n",
            "    addq %rdx, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos, mut lv) = build(asm);
        let n = line_of(&store, "movq $2, %rdx");
        edit(&mut store, &mut infos, n, "movq %r11, %rdx");
        assert!(!lv.try_refresh_span(&store, &infos, n, n));
        lv.refresh_span(&store, &infos, n, n);
        assert_same_as_fresh(&store, &infos, &lv);
        let label = line_of(&store, ".Lnext:");
        assert!(!lv.try_refresh_span(&store, &infos, n, label));
    }

    #[test]
    fn refresh_span_refuses_a_newly_structural_replacement() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "# LCCC_RET_RDX 0\n",
            "    movq %rdi, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos, mut lv) = build(asm);
        let n = line_of(&store, "movq %rdi, %rax");
        assert!(!infos[n].structural);
        edit(&mut store, &mut infos, n, ".Lfresh:");
        assert!(infos[n].structural, "replace_line must refresh the cache");
        assert!(
            !lv.try_refresh_span(&store, &infos, n, n),
            "the pre-edit liveness solution cannot be spliced across a new label"
        );
    }

    /// The memo rotates on whichever bound is hit first, and the line bound is
    /// what keeps retention proportional to bytes: 4096 entries of 20k-line
    /// functions are 80M lines, far past the budget, so they must not fit.
    #[test]
    fn fn_cache_rotation_bounds_lines_not_only_entries() {
        assert!(
            !fn_cache_gen_full(0, 0, usize::MAX),
            "an empty generation admits any single entry"
        );
        assert!(
            !fn_cache_gen_full(1, FN_CACHE_LINE_BUDGET - 16, 16),
            "an entry that exactly fills the line budget is admitted"
        );
        assert!(
            fn_cache_gen_full(1, FN_CACHE_LINE_BUDGET - 16, 17),
            "one line past the budget rotates the generation"
        );
        assert!(fn_cache_gen_full(FN_CACHE_CAP, 0, 16), "the entry cap rotates the generation");
        assert!(
            !fn_cache_gen_full(FN_CACHE_CAP - 1, 0, 16),
            "one entry under the cap still admits"
        );
        assert!(
            fn_cache_gen_full(2, usize::MAX, 1),
            "the line total saturates instead of wrapping"
        );
    }

    fn line_of(store: &LineStore, needle: &str) -> usize {
        (0..store.len())
            .find(|&i| store.get(i).contains(needle))
            .expect("line")
    }

    /// H1 (PR #628 follow-up): a thunk-extern call reads the register named
    /// by the SYMBOL SUFFIX, not a hardcoded %r10. The codegen's own form
    /// stages in %r10 today (calls.rs), but the contract of the thunk ABI
    /// (objtool --retpoline) is per-register, and the peephole's authority
    /// must model the text it is handed, not the current emitter's habits.
    #[test]
    fn thunk_call_reads_suffix_register_not_r10() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call __x86_indirect_thunk_rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let n = line_of(&store, "call __x86_indirect_thunk_rax");
        let reads = call_site_reads(&store, &infos, n);
        assert!(
            reads & (1 << 0) != 0,
            "%rax is the thunk target: {reads:#b}"
        );
        assert!(
            reads & (1 << 10) == 0,
            "%r10 is NOT read by the rax thunk: {reads:#b}"
        );
        // And the shared window verdict keeps the target register live
        // across the call (a dead-write pass must not delete its staging).
        assert_eq!(call_window_verdict(&store, &infos, n, 0), Some(false));
        assert_eq!(
            call_window_verdict(&store, &infos, n, 10),
            Some(true),
            "r10 is caller-saved and unread: clobbered"
        );
    }

    /// The r10 form keeps its historical read, and an UNRECOGNISED thunk
    /// suffix stays fail-closed on the %r10 model (the register the
    /// call-site lowering stages targets in).
    #[test]
    fn thunk_call_r10_and_unrecognised_suffix_fail_closed() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call __x86_indirect_thunk_r10\n",
            "    call __x86_indirect_thunk_zz9\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let n10 = line_of(&store, "thunk_r10");
        let reads10 = call_site_reads(&store, &infos, n10);
        assert!(reads10 & (1 << 10) != 0, "%r10 read: {reads10:#b}");
        assert_eq!(call_window_verdict(&store, &infos, n10, 10), Some(false));
        let nzz = line_of(&store, "thunk_zz9");
        let readszz = call_site_reads(&store, &infos, nzz);
        assert!(
            readszz & (1 << 10) != 0,
            "unrecognised suffix keeps the fail-closed %r10 model: {readszz:#b}"
        );
    }

    /// The jmp thunk forms the codegen ACTUALLY emits today (`jmp
    /// __x86_indirect_thunk_rax` computed goto, `..._rdx` PIC switch
    /// dispatch) classify as JmpIndirect whose reg_refs name the dispatch
    /// register — the read every textual consumer was blind to.
    #[test]
    fn thunk_jump_classification_names_the_dispatch_register() {
        for (line, fam) in [
            ("    jmp __x86_indirect_thunk_rax", 0u8),
            ("    jmp __x86_indirect_thunk_rdx", 2),
            ("    jmp __x86_indirect_thunk_r10", 10),
        ] {
            let info = classify_line(line);
            assert!(matches!(info.kind, LineKind::JmpIndirect), "{line}");
            assert!(
                info.reg_refs & (1 << fam) != 0,
                "{line}: family {fam} must be in reg_refs"
            );
        }
    }

    /// A PIC switch dispatched through a thunk-extern retpoline resolves
    /// to its table targets (the staging chain is identical to the `jmpq
    /// *%rT` form), so the enclosing function stays ANALYSABLE — every
    /// liveness-driven peephole used to be lost in such functions — and
    /// the dispatch register's staging write is live at the jmp (the
    /// thunk reads it; deleting the `addq %rcx, %rdx` would jump through
    /// garbage).
    #[test]
    fn pic_switch_thunk_dispatch_is_analysable_and_keeps_staging_live() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %edi, %eax\n",
            "    leaq .Ljt0(%rip), %rcx\n",
            "    movslq (%rcx,%rax,4), %rdx\n",
            "    addq %rcx, %rdx\n",
            "    jmp __x86_indirect_thunk_rdx\n",
            ".LBB0:\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB1:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".Ljt0:\n",
            "    .long .LBB0 - .Ljt0\n",
            "    .long .LBB1 - .Ljt0\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let jmp = line_of(&store, "jmp __x86_indirect_thunk_rdx");
        assert!(
            lv.live_after(jmp, 2).is_some(),
            "the function must be analysable despite the thunk dispatch"
        );
        let add = line_of(&store, "addq %rcx, %rdx");
        assert_eq!(
            lv.live_after(add, 2),
            Some(true),
            "the thunk reads %rdx: the staging write stays live"
        );
    }

    #[test]
    fn string_instruction_predicate_accepts_only_real_string_ops() {
        for ok in [
            "rep movsb",
            "rep movsq",
            "rep stosb",
            "rep stosq",
            "repne scasb",
            "repe cmpsb",
            "movsb",
            "stosq",
            "lodsb",
            "movsl",
            "rep stosl",
        ] {
            assert!(is_string_instruction(ok), "{ok}");
        }
        for no in [
            "movsbl %al, %eax",
            "movslq %eax, %rax",
            "movsd %xmm0, (%rdi)",
            "movss %xmm1, %xmm0",
            "movsx %al, %eax",
            "movq %rax, %rcx",
            "cmpq $1, %rax",
            "repz ret",
            "movsxd %eax, %rax",
        ] {
            assert!(!is_string_instruction(no), "{no}");
        }
    }

    #[test]
    fn rep_movsb_keeps_count_and_pointers_live() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq $4096, %rcx\n",
            "    rep movsb\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let n = line_of(&store, "movq $4096, %rcx");
        assert_eq!(
            lv.live_after(n, 1),
            Some(true),
            "%rcx must be live into rep movsb"
        );
        assert_eq!(
            lv.live_after(n, 6),
            Some(true),
            "%rsi must be live into rep movsb"
        );
        assert_eq!(
            lv.live_after(n, 7),
            Some(true),
            "%rdi must be live into rep movsb"
        );
    }

    #[test]
    fn rep_stosb_keeps_fill_value_live() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %eax, %eax\n",
            "    movq $512, %rcx\n",
            "    rep stosb\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let n = line_of(&store, "xorl %eax, %eax");
        assert_eq!(
            lv.live_after(n, 0),
            Some(true),
            "%rax must be live into rep stosb"
        );
    }

    /// `mulss`/`divsd`/... write only their explicit `%xmm` destination.
    /// `parse_dest_reg_fast` used to prefix-match them as the one-operand
    /// integer `mul`/`div` (implicit `%rax:%rdx` write), which made the
    /// realigned-alloca base below "dead" after `andq` and let
    /// dead-flag-write elimination delete the `addq $63`/`andq $-64` of
    /// every `-mno-avx` over-aligned vector store (wrong-address store).
    #[test]
    fn sse_scalar_arith_is_not_an_implicit_rax_write() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $152, %rsp\n",
            "    movss .LCFP_0(%rip), %xmm2\n",
            "    xorl %r11d, %r11d\n",
            ".LBB1:\n",
            "    cmpq $16, %r11\n",
            "jge .LBB3\n",
            ".LBB2:\n",
            "    leaq 0(,%r11,4), %r10\n",
            "    leaq 16(%rsp), %rax\n",
            "    addq $63, %rax\n",
            "    andq $-64, %rax\n",
            "    pxor %xmm3, %xmm3\n",
            "    cvtsi2ssq %r11, %xmm3\n",
            "    mulss %xmm2, %xmm3\n",
            "    movss %xmm3, (%r10, %rax)\n",
            "    divsd %xmm2, %xmm4\n",
            "    addq $1, %r11\n",
            "    cmpq $16, %r11\n",
            "jl .LBB2\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        for (line, rax_written) in [
            ("mulss %xmm2, %xmm3", false),
            ("divsd %xmm2, %xmm4", false),
            ("andq $-64, %rax", true),
        ] {
            let n = line_of(&store, line);
            assert_eq!(lv.effect[n].1 & 1 != 0, rax_written, "writes of `{line}`");
        }
        let idx = line_of(&store, "andq $-64, %rax");
        assert_eq!(lv.live_after(idx, 0), Some(true), "SIB base must stay live");
        // The one-operand integer forms keep their implicit accumulator write.
        for m in ["mulq %rcx", "divl %ecx", "idivq %r8", "mul %rbx"] {
            assert_eq!(parse_dest_reg_fast(m), 0, "{m}");
        }
        // The SSE forms classify their explicit `%xmm` destination (a
        // non-GP family), never the accumulator.
        for m in [
            "mulss %xmm2, %xmm3",
            "divpd %xmm1, %xmm0",
            "mulsd (%rax), %xmm1",
        ] {
            assert!(parse_dest_reg_fast(m) > REG_GP_MAX, "{m}");
        }
    }

    #[test]
    fn loop_body_write_is_dead_when_only_the_prologue_reads_it() {
        // %rdi is read by the prologue copy, then written (never read) in the
        // loop body: the write is dead even though the family IS mentioned
        // elsewhere in the function.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rsi\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpl %edx, %ebx\n",
            "    jae .LBB3\n",
            ".LBB2:\n",
            "    leaq 0(,%rbx,4), %rdi\n",
            "    movl (%rsi,%rbx,4), %eax\n",
            "    addl $1, %ebx\n",
            "    jmp .LBB1\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "leaq 0(,%rbx,4)");
        assert_eq!(lv.live_after(idx, 7), Some(false), "rdi must be dead");
        let idx_copy = line_of(&store, "movq %rdi, %rsi");
        assert_eq!(lv.live_after(idx_copy, 6), Some(true), "rsi live into loop");
    }

    #[test]
    fn value_live_across_the_back_edge_is_not_dead() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    addl %ebx, %eax\n",
            "    addl $1, %ebx\n",
            "    cmpl $10, %ebx\n",
            "    jl .LBB1\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "addl $1, %ebx");
        assert_eq!(lv.live_after(idx, 3), Some(true), "rbx live via back edge");
    }

    #[test]
    fn return_value_is_live_at_ret() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl $7, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "movl $7, %eax");
        assert_eq!(lv.live_after(idx, 0), Some(true));
    }

    #[test]
    fn argument_registers_are_live_into_a_call() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rbx, %rdi\n",
            "    call bar\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "movq %rbx, %rdi");
        assert_eq!(lv.live_after(idx, 7), Some(true));
    }

    /// Families (REG_NAMES order): %r8 = 8, %r14 = 14, %rbx = 3.
    fn upper0(asm: &str, label: &str, fam: RegId) -> bool {
        let (store, infos, _lv) = build(asm);
        FileLiveness::upper32_zero_into_label(&store, &infos, line_of(&store, label), fam)
    }

    /// A function with an early-exit diamond: `.Lj` is reached by
    /// fallthrough after `{fall}` and by the jump after `{taken}`.
    fn diamond(taken: &str, fall: &str) -> String {
        format!(
            "f:\n.cfi_startproc\n{taken}    testl %eax, %eax\n    je .Lj\n{fall}.Lj:\n    xorq %r14, %r8\n    ret\n.cfi_endproc\n"
        )
    }

    #[test]
    fn upper_zero_crosses_a_label_when_every_path_proves_it() {
        // zstd_count's shape: the 32-bit definer sits in a predecessor.
        let asm = diamond("    andl $1048319, %r14d\n", "    addl $1, %eax\n");
        assert!(upper0(&asm, ".Lj:", 14));
        // Both arms define it, each with a 32-bit write.
        let asm = diamond("    movl %edi, %r14d\n", "    leal 1(%rsi), %r14d\n");
        assert!(upper0(&asm, ".Lj:", 14));
    }

    #[test]
    fn upper_zero_refuses_when_any_path_does_not_prove_it() {
        // A 64-bit definer on the fallthrough arm.
        let asm = diamond("    andl $255, %r14d\n", "    movq (%rdi), %r14\n");
        assert!(!upper0(&asm, ".Lj:", 14));
        // Never written: the SysV entry value has an unspecified upper half.
        let asm = diamond("", "    addl $1, %eax\n");
        assert!(!upper0(&asm, ".Lj:", 14));
        // Conditional writers leave the old upper half on the untaken path.
        let asm = diamond("    andl $255, %r14d\n", "    cmovneq %rsi, %r14\n");
        assert!(!upper0(&asm, ".Lj:", 14));
        let asm = diamond("    andl $255, %r14d\n", "    cmovnel %esi, %r14d\n");
        assert!(!upper0(&asm, ".Lj:", 14));
        // An implicit-register line is a uniform stop (xchg writes BOTH
        // operands; the classified destination alone would miss %r14).
        let asm = diamond("    andl $255, %r14d\n", "    xchgq %r14, %rsi\n");
        assert!(!upper0(&asm, ".Lj:", 14));
        // A call is a uniform stop, even for a callee-saved family.
        let asm = diamond("    movl $1, %ebx\n", "    call g\n");
        assert!(!upper0(&asm, ".Lj:", 3));
        // A partial write leaves bits 8..63 as they were.
        let asm = diamond("    movq %rdi, %r14\n", "    movb $1, %r14b\n");
        assert!(!upper0(&asm, ".Lj:", 14));
    }

    #[test]
    fn upper_zero_needs_the_back_edge_to_prove_it_too() {
        // fill_buffer's shape: zeroed on entry, 32-bit increment on the latch.
        let lp = |latch: &str| {
            format!(
                "f:\n.cfi_startproc\n    xorl %r8d, %r8d\n.Lh:\n    movb %al, (%r10,%r8)\n{latch}    cmpl $64, %r8d\n    jb .Lh\n    ret\n.cfi_endproc\n"
            )
        };
        assert!(upper0(&lp("    addl $1, %r8d\n"), ".Lh:", 8));
        assert!(!upper0(&lp("    addq $1, %r8\n"), ".Lh:", 8));
    }

    #[test]
    fn upper_zero_converges_in_sweeps_bounded_by_loop_nesting() {
        // 2000 straight-line lines between the definer and a loop header,
        // then a latch: the solve must converge in a handful of sweeps (a
        // Jacobi iteration would need ~2000 and give up at the cap).
        let mut asm = String::from("f:\n.cfi_startproc\n    andl $255, %r14d\n");
        for _ in 0..2000 {
            asm.push_str("    addl $1, %eax\n");
        }
        asm.push_str(
            ".Lh:\n    xorq %r14, %r8\n    subl $1, %ecx\n    jne .Lh\n    ret\n.cfi_endproc\n",
        );
        assert!(upper0(&asm, ".Lh:", 14));
        let sweeps = UPPER_SWEEPS.with(|c| c.get());
        assert!(sweeps <= 3, "{sweeps} sweeps");
    }

    #[test]
    fn upper_zero_refuses_unanalysable_control_flow() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    andl $255, %r14d\n",
            "    testl %eax, %eax\n",
            "    je .Lj\n",
            "    jmpq *%rdx\n",
            ".Lj:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        assert!(!upper0(asm, ".Lj:", 14));
        // Outside every function (no CFI frame): no CFG, no answer.
        assert!(!upper0("    andl $255, %r14d\n.Lj:\n    ret\n", ".Lj:", 14));
    }

    #[test]
    fn fallthrough_fast_reject_is_exact_on_its_cases() {
        let q = |asm: &str| {
            let (store, infos, _lv) = build(asm);
            let label = line_of(&store, ".Lj:");
            FileLiveness::fallthrough_path_refutes_upper32_zero(&store, &infos, label, 14)
        };
        // Kill on the fallthrough run: refuted without a solve.
        assert!(q(&diamond(
            "    andl $255, %r14d\n",
            "    movq (%rdi), %r14\n"
        )));
        assert!(q(&diamond("    andl $255, %r14d\n", "    call g\n")));
        // The run reaches the function entry with no write: refuted.
        assert!(q(&diamond("", "")));
        // A proving write on the run, or another label first: inconclusive
        // (the taken edge still has to be checked by the solve).
        assert!(!q(&diamond(
            "    movq (%rdi), %r14\n",
            "    movl %esi, %r14d\n"
        )));
        assert!(!q(&diamond(
            "    andl $255, %r14d\n",
            ".Lm:\n    addl $1, %eax\n"
        )));
        // ... and the solve then refutes through the TAKEN edge.
        assert!(!upper0(
            &diamond("    movq (%rdi), %r14\n", "    movl %esi, %r14d\n"),
            ".Lj:",
            14
        ));
    }

    #[test]
    fn may_write_mask_equals_writes_family_per_family() {
        for t in [
            "movl %esi, %r14d",
            "movq (%rdi), %r14",
            "addq $1, %r14",
            "movb $1, %r14b",
            "cqto",
            "idivq %r11",
            "cltq",
            "mulq %rcx",
            "rep stosb",
            "movsq",
            "xchgq %r14, %rsi",
            "cmpxchgq %rbx, (%rdi)",
            "lock xaddl %eax, (%rdi)",
            "setne %al",
            "popq %rbx",
            "cpuid",
            "syscall",
            "imull $3, %eax, %r9d",
            "movl %eax, (%rdi)",
            "testq %r14, %r14",
            "shldq %cl, %rax, %rdx",
            "rdtsc",
        ] {
            let info = classify_line(&format!("    {t}"));
            let mut want = 0u16;
            for fam in 0..=REG_GP_MAX {
                if super::super::helpers::writes_family(&info, t, fam) {
                    want |= 1 << fam;
                }
            }
            assert_eq!(may_write_families(&info, t), want, "{t}");
        }
    }

    #[test]
    fn cached_transfer_is_keyed_on_the_whole_line_state() {
        let t = "movl %esi, %r14d";
        let live = classify_line(&format!("    {t}"));
        let mut pinned = live;
        pinned.pinned = true;
        // Same text, different state: neither answer may leak into the other.
        for _ in 0..2 {
            assert_eq!(
                upper32_zero_transfer_cached(&live, t),
                upper32_zero_transfer(&live, t)
            );
            assert_eq!(upper32_zero_transfer_cached(&pinned, t), (0, 0));
        }
        assert_eq!(upper32_zero_transfer_cached(&live, t).1, 1 << 14);
    }

    #[test]
    fn prove_prefilter_is_a_necessary_condition() {
        // The implication the prefilter relies on, over proving and
        // non-proving writes alike: s_write_proves => prefilter.
        let lines = [
            "movl %esi, %r14d",
            "andl $255,%r14d",
            "incl %r14d",
            "cmovnel %esi, %r14d",
            "xorq %r14, %r14",
            "subq %r14, %r14",
            "movq $5, %r14",
            "movq $-1, %r14",
            "movabsq $5, %r14",
            "movq (%rdi), %r14",
            "addq $1, %r14",
            "movl %r14d, %esi",
            "movw %si, %r14w",
            "leal 1(%rsi,%rdi,2), %r14d",
        ];
        for t in lines {
            let proves = super::super::relay_and_lea::s_write_proves_upper_zero(t, 14);
            assert!(!proves || write_may_prove_upper_zero(t, 14), "{t}");
        }
        // The cheap rejections it exists for.
        for t in [
            "addq $1, %r14",
            "movl %r14d, %esi",
            "movw %si, %r14w",
            "imulq %rax, %r14",
        ] {
            assert!(!write_may_prove_upper_zero(t, 14), "{t}");
        }
    }

    #[test]
    fn upper_zero_memo_cannot_serve_a_stale_fact() {
        let asm = diamond("    andl $255, %r14d\n", "    addl $1, %eax\n");
        let (mut store, mut infos, _lv) = build(&asm);
        let label = line_of(&store, ".Lj:");
        assert!(FileLiveness::upper32_zero_into_label(
            &store, &infos, label, 14
        ));
        let n = line_of(&store, "addl $1, %eax");
        edit(&mut store, &mut infos, n, "movq (%rdi), %r14");
        assert!(!FileLiveness::upper32_zero_into_label(
            &store, &infos, label, 14
        ));
    }

    #[test]
    fn relay_proof_uses_the_cross_block_fact() {
        // End to end through `upper32_zero_at`, the P3 relay gate: the copy
        // sits right after the label; the definer is a predecessor.
        let ok = diamond("    andl $255, %r14d\n", "    addl $1, %eax\n")
            .replace(".Lj:\n", ".Lj:\n    movl %r14d, %esi\n");
        let (store, infos, _lv) = build(&ok);
        let copy = line_of(&store, "movl %r14d, %esi");
        assert!(super::super::relay_and_lea::upper32_zero_at(
            &store, &infos, copy, 14
        ));
        let bad = diamond("    andl $255, %r14d\n", "    notq %r14\n")
            .replace(".Lj:\n", ".Lj:\n    movl %r14d, %esi\n");
        let (store, infos, _lv) = build(&bad);
        let copy = line_of(&store, "movl %r14d, %esi");
        assert!(!super::super::relay_and_lea::upper32_zero_at(
            &store, &infos, copy, 14
        ));
    }

    #[test]
    fn indirect_jump_makes_the_function_unanalysable() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl $7, %eax\n",
            "    jmpq *%rdx\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "movl $7, %eax");
        assert_eq!(lv.live_after(idx, 0), None);
    }
    /// The marker block is parsed as a whole: on a variadic static-chain
    /// call the chain marker sits third (`[call][VA][ARGS][CHAIN]`), past the
    /// two-line window the oracle used to search, and the %r10 staging write
    /// then looked dead.
    #[test]
    fn chain_marker_after_va_and_args_markers_keeps_r10_live() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rbx, %r10\n",
            "    movb $0, %al\n",
            "    call bar\n",
            "    # LCCC_VA_CALL\n",
            "    # LCCC_CALL_ARGS 0\n",
            "    # LCCC_CHAIN_CALL\n",
            "    # LCCC_CALL_FP 0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, infos, lv) = build(asm);
        let idx = line_of(&store, "movq %rbx, %r10");
        assert_eq!(
            lv.live_after(idx, 10),
            Some(true),
            "the chain call reads %r10"
        );
        let call = line_of(&store, "call bar");
        let m = call_markers(&store, &infos, call);
        assert!(
            m.variadic && m.chain && m.gp_args == 0 && m.ipa_keep == 0,
            "{m:?}"
        );
    }

    /// The marker block of a transfer ends at a label. A sibling call that
    /// opens the next block publishes its suppressed call's markers right
    /// after that label, ahead of its own `jmp`; reading past `.LBB2:` made
    /// `jmp exit@PLT` (one argument) inherit `jmp abort@PLT`'s
    /// `# LCCC_CALL_ARGS 0`, and the staging of exit's argument died
    /// (gcc.c-torture 981130-1 and 980526-2 exited with the wrong status).
    #[test]
    fn marker_block_ends_at_a_label() {
        let asm = concat!(
            "check:\n",
            ".cfi_startproc\n",
            "    cmpl %esi, %edi\n",
            "    jne .LBB2\n",
            ".LBB1:\n",
            "    xorl %edi, %edi\n",
            "    # LCCC_CALL_ARGS 1\n",
            "    # LCCC_CALL_FP 0\n",
            "    jmp exit@PLT\n",
            "    # LCCC_TAIL_CALL\n",
            "    # LCCC_CALL_ARGS 1\n",
            "    # LCCC_CALL_FP 0\n",
            ".LBB2:\n",
            "    # LCCC_CALL_ARGS 0\n",
            "    # LCCC_CALL_FP 0\n",
            "    jmp abort@PLT\n",
            "    # LCCC_TAIL_CALL\n",
            "    # LCCC_CALL_ARGS 0\n",
            "    # LCCC_CALL_FP 0\n",
            ".cfi_endproc\n",
        );
        let (store, infos, lv) = build(asm);
        let jmp = line_of(&store, "jmp exit@PLT");
        assert_eq!(call_markers(&store, &infos, jmp).gp_args, 1);
        let xor = line_of(&store, "xorl %edi, %edi");
        assert_eq!(lv.live_after(xor, 7), Some(true), "exit reads %rdi");
        // A plain call ending its block is bounded the same way: with no
        // marker of its own before the label it keeps the fail-closed six.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $3, %esi\n",
            "    call g\n",
            ".LBB1:\n",
            "    # LCCC_CALL_ARGS 0\n",
            "    jmp h@PLT\n",
            "    # LCCC_TAIL_CALL\n",
            "    # LCCC_CALL_ARGS 0\n",
            ".cfi_endproc\n",
        );
        let (store, infos, lv) = build(asm);
        let call = line_of(&store, "call g");
        assert_eq!(call_markers(&store, &infos, call).gp_args, 6);
        let mov = line_of(&store, "movl $3, %esi");
        assert_eq!(lv.live_after(mov, 6), Some(true), "g may read %rsi");
    }

    /// `# LCCC_IPA_KEEP`: a caller-saved family the caller keeps live across
    /// the call (the callee provably never writes it) is READ by the call in
    /// the oracle's model, so its definition before the call stays live.
    /// Without the marker the ABI clobber kills it, and an illegible mask
    /// fails closed to every caller-saved family.
    #[test]
    fn ipa_keep_marker_carries_a_caller_saved_value_across_the_call() {
        let with = |keep: &str| {
            format!(
                "foo:\n.cfi_startproc\n    movq %rbx, %r8\n    call bar\n\
                 {keep}    # LCCC_CALL_ARGS 0\n    movq %r8, %rax\n    ret\n.cfi_endproc\n"
            )
        };
        let r8 = 8;
        let asm = with("    # LCCC_IPA_KEEP 0x0100\n");
        let (store, infos, lv) = build(&asm);
        let idx = line_of(&store, "movq %rbx, %r8");
        assert_eq!(lv.live_after(idx, r8), Some(true));
        let call = line_of(&store, "call bar");
        assert_eq!(call_markers(&store, &infos, call).ipa_keep, 1 << r8);
        assert_eq!(call_window_verdict(&store, &infos, call, r8), Some(false));
        let asm = with("");
        let (store, _infos, lv) = build(&asm);
        let idx = line_of(&store, "movq %rbx, %r8");
        assert_eq!(
            lv.live_after(idx, r8),
            Some(false),
            "ABI clobber without the marker"
        );
        let asm = with("    # LCCC_IPA_KEEP r8\n");
        let (store, infos, _lv) = build(&asm);
        let call = line_of(&store, "call bar");
        assert_eq!(call_markers(&store, &infos, call).ipa_keep, CALLER_SAVED);
        // Callee-saved bits in a mask are meaningless and dropped.
        let asm = with("    # LCCC_IPA_KEEP 0xf108\n");
        let (store, infos, _lv) = build(&asm);
        let call = line_of(&store, "call bar");
        assert_eq!(call_markers(&store, &infos, call).ipa_keep, 1 << r8);
    }

    #[test]
    fn syscall_kill_from_the_oracle_ends_a_loop_carried_def() {
        // `syscall` destroys %r11 — knowledge only the central oracle has
        // (the old hand arms wrote nothing for it).  Without the kill, the
        // definition flows around the back edge to the read at the loop
        // top and looks live; with it, the definition is provably dead.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpl %edi, %ebx\n",
            "    jae .LBB3\n",
            ".LBB2:\n",
            "    movl %r11d, %ecx\n",
            "    movl $7, %r11d\n",
            "    syscall\n",
            "    addl $1, %ebx\n",
            "    jmp .LBB1\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "movl $7, %r11d");
        assert_eq!(
            lv.live_after(idx, 11),
            Some(false),
            "syscall's implicit %r11 write must kill the loop-carried def"
        );
    }

    #[test]
    fn cltd_kill_from_the_oracle_cuts_the_loop_carried_def() {
        // `cltd` (emitted for every 32-bit division on x86-64) was absent
        // from the hand arms: its RDX write was never modelled as a kill.
        // The definition below can only reach the loop-top read by flowing
        // through `cltd` — with the oracle's kill it is provably dead;
        // without it, the back edge makes it look live.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpl %edi, %ebx\n",
            "    jae .LBB3\n",
            "    movl %edx, %r10d\n",
            "    movl %esi, %edx\n",
            "    cltd\n",
            "    addl $1, %ebx\n",
            "    jmp .LBB1\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let idx = line_of(&store, "movl %esi, %edx");
        assert_eq!(
            lv.live_after(idx, 2),
            Some(false),
            "cltd's implicit %rdx write must kill the loop-carried def"
        );
    }

    #[test]
    fn ret_rdx_marker_is_authoritative() {
        // The epilogue reads `%rdx` (`movzbl %dl`), which defeats the
        // tail-block detector — yet the marker says the function returns an
        // `int`, so `%rdx` is dead after the copy.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    movzbl %dl, %r11d\n",
            "    andl $127, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let n = line_of(&store, "movzbl %dl, %r11d");
        assert_eq!(lv.live_after(n, 2), Some(false));
        // And the `1` spelling keeps `%rdx` live (an i128 return).
        let asm = asm.replace("# LCCC_RET_RDX 0", "# LCCC_RET_RDX 1");
        let (store, _infos, lv) = build(&asm);
        let n = line_of(&store, "movzbl %dl, %r11d");
        assert_eq!(lv.live_after(n, 2), Some(true));
    }

    #[test]
    fn ret_rdx_falls_back_to_tail_inference_without_marker() {
        // No marker: the tail-block detector decides. A `%rdx` mention in
        // the tail keeps the register conservatively live (whether or not
        // the function "really" returns an `int` — unknowable from text).
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl %dl, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let n = line_of(&store, "movzbl %dl, %r11d");
        assert_eq!(lv.live_after(n, 2), Some(true));
        // A clean tail still infers rax-only without any marker.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, _infos, lv) = build(asm);
        let n = line_of(&store, "movl %r11d, %eax");
        assert_eq!(lv.live_after(n, 2), Some(false));
    }

    /// The memo key must cover `LineInfo` state that the *text* does not
    /// encode. Passes force `has_indirect_mem` / `rbp_offset` / `kind` on lines
    /// whose text they leave alone (`passes/mod.rs`), and the solver's
    /// store/load modelling reads exactly those fields -- so a key built from
    /// text + `pinned` + `reg_refs` could map two different programs onto one
    /// entry and answer liveness for the wrong one.
    #[test]
    fn memo_key_covers_line_info_the_text_does_not_encode() {
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    movq %rdi, %rax\n",
            "    movq 8(%rax), %rcx\n",
            "    addq %rcx, %rdx\n",
            "    movq %rcx, 16(%rax)\n",
            "    movq -8(%rbp), %r8\n",
            "    movq %r8, -16(%rbp)\n",
            "    testq %rdx, %rdx\n",
            "    je .L1\n",
            "    movq %rdx, %r9\n",
            "    addq $8, %r9\n",
            "    movq %r9, 24(%rax)\n",
            ".L1:\n",
            "    movq %rcx, %r10\n",
            "    subq %r8, %r10\n",
            "    movq %r10, 32(%rax)\n",
            "    ret\n.cfi_endproc\n",
        );
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let base = FileLiveness::hash_function(&store, &infos, 0, store.len());

        // Same text, one classification field changed, exactly as a fold does.
        for mutate in [
            (|i: &mut LineInfo| i.has_indirect_mem = !i.has_indirect_mem) as fn(&mut LineInfo),
            |i: &mut LineInfo| i.rbp_offset = i.rbp_offset.wrapping_add(8),
        ] {
            let mut altered = infos.clone();
            if let Some(info) = altered.iter_mut().find(|i| !i.is_nop()) {
                mutate(info);
            }
            assert_ne!(
                base,
                FileLiveness::hash_function(&store, &altered, 0, store.len()),
                "a field the solver reads but the text does not spell must change the key"
            );
        }

        // `pinned` and the nop mark are decisions of earlier passes, not text.
        let mut pinned = infos.clone();
        if let Some(info) = pinned.iter_mut().find(|i| !i.is_nop()) {
            info.pinned = true;
        }
        assert_ne!(
            base,
            FileLiveness::hash_function(&store, &pinned, 0, store.len())
        );
    }

    /// A memo hit must be indistinguishable from a fresh solve. Two functions
    /// with identical bodies share one key; the second solve hits, and every
    /// per-line answer must match the first.
    #[test]
    fn memo_hit_matches_a_fresh_solve() {
        let body = concat!(
            "    # LCCC_RET_RDX 0\n",
            "    # LCCC_RET_RAX 0\n",
            "    movq %rdi, %rax\n",
            "    movq 8(%rax), %rcx\n",
            "    addq %rcx, %rdx\n",
            "    movq %rcx, 16(%rax)\n",
            "    movq %rcx, %r8\n",
            "    subq %rdx, %r8\n",
            "    testq %r8, %r8\n",
            "    je .L1\n",
            "    movq %r8, %r9\n",
            "    addq $8, %r9\n",
            "    movq %r9, 24(%rax)\n",
            ".L1:\n",
            "    movq %r8, %r10\n",
            "    addq %r9, %r10\n",
            "    movq %r10, 32(%rax)\n",
            "    ret\n",
        );
        let asm = format!(
            "g1:\n.cfi_startproc\n{body}.cfi_endproc\ng2:\n.cfi_startproc\n{body}.cfi_endproc\n"
        );
        let store = LineStore::new(asm);
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let starts: Vec<usize> = (0..store.len())
            .filter(|&i| store.get(i).trim_ascii() == ".cfi_startproc")
            .collect();
        assert_eq!(starts.len(), 2);
        let ends: Vec<usize> = (0..store.len())
            .filter(|&i| store.get(i).trim_ascii() == ".cfi_endproc")
            .collect();
        assert_eq!(ends.len(), 2);

        MEMO_HITS.with(|c| c.set(0));
        let lv = FileLiveness::new(&store, &infos);
        assert_eq!(
            MEMO_HITS.with(|c| c.get()),
            1,
            "the second identical function must be served from the memo"
        );
        let (a, b) = (starts[0]..ends[0], starts[1]..ends[1]);
        assert_eq!(lv.live_out[a.clone()], lv.live_out[b.clone()]);
        assert_eq!(lv.live_in[a.clone()], lv.live_in[b.clone()]);
        assert_eq!(lv.known[a.clone()], lv.known[b.clone()]);
        assert_eq!(lv.structural[a.clone()], lv.structural[b.clone()]);
        assert_eq!(lv.effect[a.clone()], lv.effect[b]);
        assert_eq!(
            lv.textual_ret.contains(&starts[0]),
            lv.textual_ret.contains(&starts[1])
        );
    }

    /// The `straight_effect` memo answers exactly what the uncached
    /// derivation answers, on a line set that walks every branch of it
    /// (pure write at full and partial width, load, self-xor zeroing and a
    /// non-self xor, RMW, xchg, string op, unknown mnemonic, push/pop, the
    /// implicit-operand kills `cltd`/`syscall`, the partial implicit write
    /// `lahf`, cmov).  The second round must be served from the memo, line
    /// for line: the hit counter proves it rather than inferring it.
    #[test]
    fn straight_effect_memo_answers_like_the_uncached_derivation() {
        let asm = concat!(
            "    movq %rdi, %rax\n",
            "    movl %esi, %eax\n",
            "    movb %sil, %al\n",
            "    movq (%rax), %rax\n",
            "    xorl %eax, %eax\n",
            "    xorl %ecx, %eax\n",
            "    addq %rdx, %rcx\n",
            "    xchgq %rax, %rbx\n",
            "    rep movsb\n",
            "    frobnicate %r8, %r9\n",
            "    pushq %rbx\n",
            "    popq %r12\n",
            "    cltd\n",
            "    syscall\n",
            "    lahf\n",
            "    cmovneq %rsi, %rdi\n",
        );
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let real: Vec<usize> = (0..store.len()).filter(|&n| !infos[n].is_nop()).collect();
        assert!(real.len() >= 16, "every fixture line must be classified");
        // Distinct texts: the first round can only miss (or hit an entry a
        // previous test on this thread left, which is equally exact).
        for round in 0..2 {
            let hits_before = EFFECT_MEMO_HITS.with(|c| c.get());
            for &n in &real {
                let t = infos[n].trimmed(store.get(n));
                let memo = FileLiveness::straight_effect(&infos, n, t);
                let raw = FileLiveness::straight_effect_uncached(&infos[n], t);
                assert_eq!(
                    (memo.reads, memo.writes),
                    (raw.reads, raw.writes),
                    "round {round}: `{t}`"
                );
            }
            if round == 1 {
                assert_eq!(
                    EFFECT_MEMO_HITS.with(|c| c.get()) - hits_before,
                    real.len() as u64,
                    "the second round must be served from the memo"
                );
            }
        }
    }

    /// The memo key covers the classification, not just the text: a pass
    /// that re-types a line without editing it (as `passes/mod.rs` does to
    /// `kind`) or widens its `reg_refs` must get the derivation for the NEW
    /// classification.  A text-only key would hand back the stale write of
    /// `%rax` here and keep a dead value's producer alive -- or, the other
    /// way round, drop a live one.
    #[test]
    fn straight_effect_memo_key_covers_kind_and_reg_refs() {
        let store = LineStore::new("    movq %rdi, %rax\n".to_string());
        let mut infos = vec![classify_line(store.get(0))];
        let t = infos[0].trimmed(store.get(0));
        assert!(matches!(infos[0].kind, LineKind::Other { dest_reg: 0 }));
        let typed = FileLiveness::straight_effect(&infos, 0, &t);
        assert_eq!(typed.writes, RAX, "`movq %rdi, %rax` kills %rax");

        infos[0].set_kind(LineKind::Other { dest_reg: REG_NONE }, t);
        let retyped = FileLiveness::straight_effect(&infos, 0, &t);
        let raw = FileLiveness::straight_effect_uncached(&infos[0], &t);
        assert_eq!((retyped.reads, retyped.writes), (raw.reads, raw.writes));
        assert_eq!(retyped.writes, 0, "no destination, no kill");

        infos[0].reg_refs |= 1 << 9;
        let widened = FileLiveness::straight_effect(&infos, 0, &t);
        let raw = FileLiveness::straight_effect_uncached(&infos[0], &t);
        assert_eq!((widened.reads, widened.writes), (raw.reads, raw.writes));
        assert_ne!(widened.reads & (1 << 9), 0, "a mentioned family is read");

        // And back: the entry was replaced, not shadowed, so the original
        // classification is still answered exactly.
        infos[0] = classify_line(store.get(0));
        let again = FileLiveness::straight_effect(&infos, 0, &t);
        assert_eq!((again.reads, again.writes), (typed.reads, typed.writes));
    }
}
