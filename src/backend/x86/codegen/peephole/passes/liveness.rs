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

use super::super::types::*;
use super::helpers::{get_dest_reg, is_read_modify_write, src_mentions_family};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// All 16 GP families.
const ALL: u16 = 0xFFFF;
/// rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8..r15 = 8..15.
const RAX: u16 = 1 << 0;
const RCX: u16 = 1 << 1;
const RDX: u16 = 1 << 2;
const RSP: u16 = 1 << 4;
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
const CALLER_SAVED: u16 =
    RAX | RCX | RDX | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11);
/// Live at `ret`: integer return value plus everything the callee must restore.
const RET_LIVE: u16 =
    RAX | RDX | (1 << 3) | (1 << 5) | RSP | (1 << 12) | (1 << 13) | (1 << 14) | (1 << 15);
/// Same, for a function that demonstrably returns a single value in `%rax`
/// (its epilogue writes the accumulator and never touches `%rdx`). Keeping
/// `%rdx` artificially live there hides every dead `movq %rax, %rdx` the
/// return-value materialisation leaves behind.
const RET_LIVE_RAX_ONLY: u16 = RET_LIVE & !RDX;

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
    let variadic = (n + 1..(n + 3).min(infos.len())).any(|k| {
        !infos[k].is_nop() && infos[k].trimmed(store.get(k)).starts_with("# LCCC_VA_CALL")
    });
    // A static-chain callee reads the chain staged in %r10 by
    // the SetStaticChain emission directly before this call;
    // `# LCCC_CHAIN_CALL` marks exactly those sites. The
    // staging write's own liveness DEPENDS on this read: without
    // it, dead-write elimination would retire the staging and
    // silently drop the chain.
    let chain = (n + 1..(n + 3).min(infos.len())).any(|k| {
        !infos[k].is_nop()
            && infos[k]
                .trimmed(store.get(k))
                .starts_with("# LCCC_CHAIN_CALL")
    });
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
    let mut gp_args: usize = 6;
    for k in n + 1..(n + 5).min(infos.len()) {
        if infos[k].is_nop() {
            continue;
        }
        let mk = infos[k].trimmed(store.get(k));
        if let Some(rest) = mk.strip_prefix("# LCCC_CALL_ARGS ") {
            if let Ok(v) = rest.trim().parse::<usize>() {
                gp_args = v.min(6);
            }
            break;
        }
        if mk.starts_with('#') || mk.starts_with('.') {
            continue; // a sibling marker or directive
        }
        break; // a real instruction ends the marker window
    }
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
    reads | retpoline_thunk_bits
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
    if t.split_whitespace()
        .nth(1)
        .is_some_and(|target| target.starts_with('.'))
    {
        return Some(false);
    }
    if CALLER_SAVED & mask != 0 {
        Some(true)
    } else {
        None
    }
}

/// Per-line liveness for a whole assembly file.
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
    /// last analysis (see `is_structural`).  `refresh_span` must know the
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

/// `CCC_VERIFY_LIVENESS_SPAN` (see `FileLiveness::assert_matches_full_analysis`).
fn verify_span_refresh() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_VERIFY_LIVENESS_SPAN").is_some())
}

/// Whether line `n` influences the analysis beyond its own read/write
/// effect: control transfers and labels (CFG edges), calls (their read set
/// depends on the marker lines after them, a local target adds an edge),
/// inline asm, comment lines (the `# LCCC_*` markers consulted by call and
/// ret modelling) and every directive except the byte-neutral annotation
/// and alignment ones (`.long`/`.quad` jump-table entries and section
/// switches are read by `resolve_jump_tables`).
fn is_structural(store: &LineStore, infos: &[LineInfo], n: usize) -> bool {
    if infos[n].is_nop() {
        return false;
    }
    match infos[n].kind {
        LineKind::Label
        | LineKind::Jmp
        | LineKind::CondJmp
        | LineKind::JmpIndirect
        | LineKind::Call
        | LineKind::Ret
        | LineKind::InlineAsm => true,
        LineKind::Directive => {
            let t = infos[n].trimmed(store.get(n));
            ![".cfi_", ".loc ", ".p2align ", ".balign ", ".align "]
                .iter()
                .any(|p| t.starts_with(p))
        }
        _ => infos[n].trimmed(store.get(n)).starts_with('#'),
    }
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
    let mut m = t;
    for p in ["repne ", "repnz ", "repe ", "repz ", "rep "] {
        if let Some(rest) = m.strip_prefix(p) {
            m = rest.trim_start();
            break;
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
        || implicit_reg_refs(t.trim().as_bytes()) != 0
}

impl FileLiveness {
    /// Compute liveness for every function in the file.
    #[expect(clippy::needless_range_loop)]
    pub(super) fn new(store: &LineStore, infos: &[LineInfo]) -> Self {
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
            if infos[i].is_nop() || !infos[i].trimmed(store.get(i)).starts_with(".cfi_startproc") {
                i += 1;
                continue;
            }
            let mut end = len;
            for n in i + 1..len {
                if !infos[n].is_nop() && infos[n].trimmed(store.get(n)).starts_with(".cfi_endproc")
                {
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
    pub(super) fn live_after_dropping_region(
        &self,
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
        let (start, end) = self.recorded_function(store, infos, idx)?;
        // Clone-and-re-derive: the neutralised solution must not leak into
        // the caller's (unmodified-program) solution, and every other field
        // keeps the function/structural bookkeeping the caller inherited.
        let mut probe = FileLiveness {
            live_out: self.live_out.clone(),
            live_in: self.live_in.clone(),
            known: self.known.clone(),
            structural: self.structural.clone(),
            effect: self.effect.clone(),
            func_of: self.func_of.clone(),
            textual_ret: self.textual_ret.clone(),
        };
        probe.reanalyse_with(store, infos, start, end, Some((lo, hi, mask)));
        probe.known[idx].then(|| probe.live_out[idx])
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
            let t = infos[n].trimmed(store.get(n));
            if t.starts_with(".cfi_startproc") {
                start = Some(n);
                break;
            }
            if t.starts_with(".cfi_endproc") {
                return;
            }
        }
        let Some(start) = start else { return };
        let mut end = len;
        for n in start + 1..len {
            if !infos[n].is_nop() && infos[n].trimmed(store.get(n)).starts_with(".cfi_endproc") {
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
    /// delimiting CFI lines are still in place.
    fn recorded_function(
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
    ///   (no line of it is `is_structural`, now or at the last analysis),
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
        if (lo..=hi).any(|p| self.structural[p] || is_structural(store, infos, p)) {
            return false;
        }
        let real = |n: usize| !infos[n].is_nop() && infos[n].kind != LineKind::Directive;
        // A call's read set comes from the marker window after it
        // (`call_site_reads` looks at most 4 lines ahead).
        if (lo.saturating_sub(5).max(start)..lo)
            .any(|p| !infos[p].is_nop() && infos[p].kind == LineKind::Call)
        {
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
            if is_structural(store, infos, n) {
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
                if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
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
                match m.trim() {
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
                match m.trim() {
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
    #[expect(clippy::needless_range_loop)]
    fn analyse_function(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
        neutralise: Option<(usize, usize, u16)>,
    ) {
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

        // ── instruction effects + successor edges ────────────────────────────
        let mut effects: Vec<Option<Effect>> = vec![None; end.saturating_sub(start)];
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); end.saturating_sub(start)];
        let mut lines: Vec<usize> = Vec::new();
        for n in start..end {
            if infos[n].is_nop() || infos[n].kind == LineKind::Directive {
                continue;
            }
            lines.push(n);
        }

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
            Self::resolve_jump_tables(store, infos, &lines, &label_index, start, end);
        for (pos, &n) in lines.iter().enumerate() {
            let t = infos[n].trimmed(store.get(n));
            let next = lines.get(pos + 1).copied();
            let rel = n - start;
            let (eff, edges) = match self.classify(
                store,
                infos,
                n,
                t,
                &resolve,
                next,
                ret_live,
                &jump_table_succs,
                start,
            ) {
                Some(v) => v,
                None => return, // unanalysable control flow: leave `known` false
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
            succs[rel] = edges;
        }

        // ── backward dataflow ────────────────────────────────────────────────
        let mut live_in: Vec<u16> = vec![0; end.saturating_sub(start)];
        let mut live_out: Vec<u16> = vec![0; end.saturating_sub(start)];
        let mut changed = true;
        let mut rounds = 0;
        while changed && rounds < 64 {
            changed = false;
            rounds += 1;
            for &n in lines.iter().rev() {
                let rel = n - start;
                let Some(eff) = effects[rel] else { continue };
                let mut out = 0u16;
                for &s in &succs[rel] {
                    out |= live_in[s - start];
                }
                if succs[rel].is_empty() {
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
            return; // did not converge (pathological CFG): stay unknown
        }

        for &n in &lines {
            self.live_out[n] = live_out[n - start];
            self.live_in[n] = live_in[n - start];
            self.known[n] = true;
            if let Some(eff) = effects[n - start] {
                self.effect[n] = (eff.reads, eff.writes);
            }
        }
        for n in start..end {
            self.structural[n] = is_structural(store, infos, n);
        }
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
                add_rest.split_once(',').map(|(b, _)| b.trim()),
                add_rest.split_once(',').map(|(_, d)| d.trim()),
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
            let parts: Vec<&str> = mem.split(',').map(str::trim).collect();
            if parts.len() != 3 || parts[0] != base_reg || parts[2] != "4" {
                continue;
            }
            let mov_dst = mov_rest[close + 1..].trim_start_matches(',').trim();
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
            if lea_dst.trim() != base_reg {
                continue;
            }
            let lea_op = lea_op.trim();
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
                    let sym = entry.split('-').next().unwrap_or(entry).trim();
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

    /// Effects and successors of one instruction, or `None` when the control
    /// transfer cannot be resolved.
    fn classify(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        n: usize,
        t: &str,
        resolve: &dyn Fn(&str) -> Option<usize>,
        next: Option<usize>,
        ret_live: u16,
        jump_table_succs: &[Option<Vec<usize>>],
        jump_table_base: usize,
    ) -> Option<(Effect, Vec<usize>)> {
        let mentioned = infos[n].reg_refs;
        let fall: Vec<usize> = next.into_iter().collect();

        if infos[n].kind == LineKind::InlineAsm
            || infos[n].pinned && infos[n].kind == LineKind::InlineAsm
        {
            return Some((
                Effect {
                    reads: ALL,
                    writes: 0,
                },
                fall,
            ));
        }

        match infos[n].kind {
            LineKind::Label => Some((
                Effect {
                    reads: 0,
                    writes: 0,
                },
                fall,
            )),
            LineKind::Ret => Some((
                Effect {
                    reads: ret_live | mentioned,
                    writes: 0,
                },
                Vec::new(),
            )),
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
                let local_target = t.split_whitespace().nth(1).and_then(resolve);
                match local_target {
                    Some(target) => {
                        let mut edges = fall;
                        if !edges.contains(&target) {
                            edges.push(target);
                        }
                        Some((Effect { reads, writes: 0 }, edges))
                    }
                    None => Some((
                        Effect {
                            reads,
                            writes: CALLER_SAVED,
                        },
                        fall,
                    )),
                }
            }
            LineKind::JmpIndirect => {
                // A resolved jump-table dispatch: successors are the table's
                // target labels (see `resolve_jump_tables`). The dispatch
                // registers are ordinary instructions classified on their
                // own lines; this line only reads the target register.
                if let Some(Some(targets)) = jump_table_succs.get(n - jump_table_base) {
                    return Some((
                        Effect {
                            reads: mentioned,
                            writes: 0,
                        },
                        targets.clone(),
                    ));
                }
                None
            }
            LineKind::Jmp => {
                let target = t.split_whitespace().nth(1)?;
                if target.starts_with('*') {
                    return None;
                }
                let idx = resolve(target)?; // tail call / foreign label
                Some((
                    Effect {
                        reads: mentioned,
                        writes: 0,
                    },
                    vec![idx],
                ))
            }
            LineKind::CondJmp => {
                let target = t.split_whitespace().nth(1)?;
                let idx = resolve(target)?;
                let mut edges = fall;
                edges.push(idx);
                Some((
                    Effect {
                        reads: mentioned,
                        writes: 0,
                    },
                    edges,
                ))
            }
            _ => Some((Self::straight_effect(infos, n, t), fall)),
        }
    }

    /// Register effect of a line that is not a control transfer, label,
    /// call, `ret` or inline asm: a function of the line's own text alone,
    /// which is what lets `refresh_span` re-derive it in isolation.
    fn straight_effect(infos: &[LineInfo], n: usize, t: &str) -> Effect {
        let mentioned = infos[n].reg_refs;
        match infos[n].kind {
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
                let dest = get_dest_reg(&infos[n]);
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
                        let dst_text = t[t.rfind(',').map(|c| c + 1).unwrap_or(0)..].trim();
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
                            rest.split_once(',').map(|(a, b)| (a.trim(), b.trim()))
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
                let iw = implicit_write_refs(t.as_bytes());
                let iwf = implicit_full_write_refs(t.as_bytes());
                reads &= !iwf; // only a full implicit write cancels an observation
                reads |= implicit_read_refs(t.as_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

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
}
