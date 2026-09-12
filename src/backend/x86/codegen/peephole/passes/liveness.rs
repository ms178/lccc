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
use super::helpers::get_dest_reg;

/// All 16 GP families.
const ALL: u16 = 0xFFFF;
/// rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8..r15 = 8..15.
const RAX: u16 = 1 << 0;
const RCX: u16 = 1 << 1;
const RDX: u16 = 1 << 2;
const RSP: u16 = 1 << 4;
/// Values an ABI-visible transfer reads without naming them: the six SysV
/// argument registers, `%rax` (variadic vector count) and `%r10` (static chain
/// for nested functions — family 10 in the peephole numbering, NOT 11 which is
/// `%r11`; getting that wrong let a relay delete the chain set-up).
const CALL_READS: u16 = RAX | RCX | RDX | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10);
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

/// Per-line liveness for a whole assembly file.
pub(super) struct FileLiveness {
    /// Registers live immediately AFTER each line, when its function could be
    /// analysed.
    live_out: Vec<u16>,
    /// Whether the line belongs to an analysable function.
    known: Vec<bool>,
}

/// Read/write sets of one instruction.
#[derive(Clone, Copy)]
struct Effect {
    reads: u16,
    writes: u16,
}

/// Mnemonics whose destination operand is written without being read.
fn is_pure_write_mnemonic(t: &str) -> bool {
    // `cmov` is deliberately absent: it only conditionally updates its
    // destination, so the old value stays live.
    (t.starts_with("mov") && !t.starts_with("movs") || t.starts_with("movs") && t.len() > 5)
        || t.starts_with("lea")
        || t.starts_with("set")
        // Always-writing bit counters. `bsf`/`bsr` are deliberately absent:
        // they leave the destination untouched when the source is zero.
        || t.starts_with("lzcnt")
        || t.starts_with("tzcnt")
        || t.starts_with("popcnt")
        // BMI2 three-operand shifts always write their destination.
        || t.starts_with("shlx")
        || t.starts_with("shrx")
        || t.starts_with("sarx")
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
            known: vec![false; len],
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
            lv.analyse_function(store, infos, i, end);
            i = end.max(i + 1);
        }
        lv
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
        for n in start..end {
            self.known[n] = false;
            self.live_out[n] = ALL;
        }
        self.analyse_function(store, infos, start, end);
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

    #[expect(clippy::needless_range_loop)]
    fn analyse_function(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        start: usize,
        end: usize,
    ) {
        let ret_live = match Self::ret_rdx_marker(store, infos, start, end) {
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
        let mut labels: Vec<(String, usize)> = Vec::new();
        for n in start..end {
            if infos[n].is_nop() {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            if infos[n].kind == LineKind::Label {
                if let Some(name) = t.strip_suffix(':') {
                    labels.push((name.to_string(), n));
                }
            }
        }
        let resolve = |name: &str| -> Option<usize> {
            labels.iter().find(|(l, _)| l == name).map(|&(_, idx)| idx)
        };

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
            Self::resolve_jump_tables(store, infos, &lines, &labels, start, end);
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
            self.known[n] = true;
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
        labels: &[(String, usize)],
        start: usize,
        end: usize,
    ) -> Vec<Option<Vec<usize>>> {
        let mut out: Vec<Option<Vec<usize>>> = vec![None; end.saturating_sub(start)];
        let resolve = |name: &str| -> Option<usize> {
            labels.iter().find(|(l, _)| l == name).map(|&(_, idx)| idx)
        };
        for (pos, &n) in lines.iter().enumerate() {
            if infos[n].kind != LineKind::JmpIndirect {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            // `jmpq *%rT` — take the target register.
            let Some(target_reg) = t.strip_prefix("jmpq *") else {
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
            if add_dst != target_reg {
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
            if mov_dst != target_reg {
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
            let Some(&(_, tbl_line)) = labels.iter().find(|(l, _)| l == table_name) else {
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
                    !infos[k].is_nop()
                        && infos[k].trimmed(store.get(k)).starts_with("# LCCC_VA_CALL")
                });
                let reads = if variadic {
                    CALL_READS | mentioned
                } else {
                    (CALL_READS & !RAX) | mentioned
                };
                Some((
                    Effect {
                        reads,
                        writes: CALLER_SAVED,
                    },
                    fall,
                ))
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
            LineKind::Push { .. } => Some((
                Effect {
                    reads: mentioned | RSP,
                    writes: RSP,
                },
                fall,
            )),
            LineKind::Pop { reg } => {
                let w = if reg != REG_NONE && reg <= REG_GP_MAX {
                    1u16 << reg
                } else {
                    0
                };
                Some((
                    Effect {
                        reads: RSP,
                        writes: w | RSP,
                    },
                    fall,
                ))
            }
            _ => {
                if is_string_instruction(t) {
                    // Implicit operands only; `rep` variants also read and
                    // update the count.  Treating the data register as both
                    // read and written is conservative for `lods`/`stos`.
                    const STRING_REGS: u16 = RAX | RCX | (1 << 6) | (1 << 7);
                    return Some((
                        Effect {
                            reads: STRING_REGS | mentioned,
                            writes: STRING_REGS,
                        },
                        fall,
                    ));
                }
                if !mnemonic_is_known(t) {
                    // Unknown: keep every mentioned register live.
                    return Some((
                        Effect {
                            reads: mentioned,
                            writes: 0,
                        },
                        fall,
                    ));
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
                        let src_part = &t[..t.rfind(',').unwrap_or(t.len())];
                        let name64 = REG_NAMES[0][dest as usize];
                        let name32 = REG_NAMES[1][dest as usize];
                        let name16 = REG_NAMES[2][dest as usize];
                        let name8 = REG_NAMES[3][dest as usize];
                        let src_reads_dest = src_part.contains(name64)
                            || src_part.contains(name32)
                            || src_part.contains(name16)
                            || src_part.contains(name8);
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
                let _ = store;
                Some((Effect { reads, writes }, fall))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>, FileLiveness) {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FileLiveness::new(&store, &infos);
        (store, infos, lv)
    }

    fn line_of(store: &LineStore, needle: &str) -> usize {
        (0..store.len())
            .find(|&i| store.get(i).contains(needle))
            .expect("line")
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
