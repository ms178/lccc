//! Generic x86-64 codegen peepholes: register move-relay elimination,
//! windowed `lea`→memory-operand folding, and copy-into-RMW-consumer
//! coalescing.
//!
//! Both passes are block-local, width-exact, and liveness-checked. They exist
//! because the accumulator-based backend routinely emits
//!
//! ```text
//!     movzbl (%rsi,%rbx), %eax
//!     movl   %eax, %r10d        <- relay copy
//!     addl   %r10d, %r8d
//! ```
//!
//! and, in unrolled pointer loops,
//!
//! ```text
//!     leaq   1(%rbx), %r10
//!     movzbl (%rbx), %r13d      <- unrelated instruction in between
//!     ...
//!     movzbl (%r10), %r14d      <- the only use of %r10
//! ```
//!
//! `local_patterns::fold_lea_into_memory_op` already folds the single-base LEA
//! form, but only when the memory operation is the IMMEDIATELY following
//! instruction and only when the temporary is dead for the rest of the function
//! (`fam_read_after`, which is forward- and path-insensitive). Unrolled loops
//! break both assumptions: the consuming load is several instructions away, and
//! the temporary register is recycled by a later LEA in the same block. The
//! windowed fold below handles exactly that shape.
//!
//! # Liveness contract (shared by both passes)
//!
//! A register family may only be dropped when it is provably dead after its
//! last rewritten use. Two independent proofs are accepted:
//!
//! 1. **Block-local write-before-read** — scanning forward from the use, the
//!    first event on the family is a FULL write (a `mov`/`lea`-class write
//!    whose source does not read the family). Any barrier (label, jump, call,
//!    push/pop, `ret`) encountered first aborts the proof, so no claim is ever
//!    made about another basic block.
//! 2. **Whole-function textual uniqueness** — inside the enclosing
//!    `.cfi_startproc`/`.cfi_endproc` range, the ONLY lines mentioning the
//!    family are the ones this transform rewrites or deletes, and the function
//!    contains no instruction with an implicit read of that family. Then no
//!    reader exists on ANY path, back edges included.
//!
//! Proof 2 is what makes the loop-tail shape of `sum8` foldable (the relay
//! target dies at the bottom of the loop body, i.e. behind the back edge);
//! proof 1 is what makes the unrolled `adler32` body foldable (the temporary is
//! immediately recycled by the next LEA).

use super::super::types::*;
use super::helpers::{
    get_dest_reg, has_implicit_reg_usage, implicit_read_reg_family, is_read_modify_write,
    replace_reg_family, writes_family,
};
use super::liveness::FileLiveness;

/// Widest window (in non-NOP instructions) searched for the consumer of a LEA.
/// Unrolled byte loops interleave 2-3 instructions between the address
/// computation and its use; 12 covers those without turning the pass
/// quadratic on large blocks.
const LEA_WINDOW: usize = 12;

/// Two-operand ALU instructions whose FIRST operand is a pure source read.
/// `imul`'s three-operand form is rejected implicitly: its operand list does
/// not match `SRC, DST` (the source split below compares the whole first
/// operand text).
const RELAY_OPS: &[&str] = &[
    "addl ", "addq ", "subl ", "subq ", "andl ", "andq ", "orl ", "orq ", "xorl ", "xorq ",
    "cmpl ", "cmpq ", "testl ", "testq ", "imull ", "imulq ", "adcl ", "adcq ", "sbbl ", "sbbq ",
];

/// `true` when `fam` is a general-purpose family this module is willing to
/// touch. `%rsp`/`%rbp` (4/5) are excluded: they carry the frame, are written
/// implicitly by push/pop/leave, and are never worth a relay.
#[inline]
pub(super) fn is_relayable_family(fam: RegId) -> bool {
    fam <= REG_GP_MAX && fam != REG_RSP && fam != REG_RBP
}

/// Is `fam` safe to *receive* a retargeted value, given per-function `%rbp`
/// knowledge from [`rbp_is_gpr_in_function`]?
///
/// `%rsp` (4) is the stack pointer on every target and is never writable by a
/// value transform. `%rbp` (5) is writable only when this function does not
/// use it as a frame pointer: lccc's default frame layout is `%rsp`-relative,
/// and the frame-pointer build (`movq %rsp, %rbp` + `N(%rbp)` slot traffic,
/// `leave`, or a `.cfi_def_cfa` rooted at rbp) makes every rbp mention part
/// of the frame contract. Callers MUST pass evidence gathered over the whole
/// enclosing function, never a local window: a single `(%rbp)` operand
/// anywhere proves the frame-pointer build.
#[inline]
pub(super) fn is_writable_family_gpr(fam: RegId, rbp_gpr: bool) -> bool {
    fam <= REG_GP_MAX && fam != REG_RSP && (fam != REG_RBP || rbp_gpr)
}

/// Scan the whole `[fstart, fend]` function range and decide whether `%rbp`
/// is an ordinary allocator-owned general register there rather than a frame
/// pointer. `true` means passes may retarget a value INTO `%rbp` (or delete a
/// copy whose only purpose was moving a value through it); `false` means rbp
/// carries the frame and must stay untouched.
///
/// Evidence treated as FRAME-POINTER (any single hit decides `false`):
/// * any parenthesised rbp operand (`N(%rbp)`, `(%rbp)`, any width) — that is
///   slot traffic or address arithmetic on the frame,
/// * the setup/restore pair (`movq %rsp, %rbp`, `movq %rbp, %rsp`) and
///   `leave`/`leaveq`,
/// * a `.cfi_def_cfa`/`.cfi_def_cfa_register` line naming rbp (the unwinder's
///   CFA is then defined by rbp, so its value is ABI-observed),
/// * a pinned or inline-assembly line mentioning rbp in any spelling — those
///   lines are opaque, so their rbp contract is unknowable.
///
/// Everything else is GPR evidence: reg-to-reg moves, ALU operands, and the
/// callee-save `pushq %rbp`/`popq %rbp` pair of an `%rsp`-relative frame
/// (which says nothing about rbp's role beyond "saved").
///
/// The scan is linear in the function and only runs for candidates that
/// actually mention rbp, so the hot no-rbp path pays one branch.
pub(super) fn rbp_is_gpr_in_function(
    store: &LineStore,
    infos: &[LineInfo],
    fstart: usize,
    fend: usize,
) -> bool {
    for n in fstart..=fend.min(infos.len().saturating_sub(1)) {
        if infos[n].is_nop() {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        // Every rule below needs an rbp spelling (each contains `bp`), a
        // `leave`/`enter` mnemonic, or a numeric `.cfi_def_cfa_register`.
        // One substring search settles the other lines -- nearly all of a
        // large function -- instead of fifteen.
        if !t.contains("bp")
            && !t.starts_with("leave")
            && !t.starts_with("enter")
            && !t.starts_with(".cfi_def_cfa_register")
        {
            continue;
        }
        // Opaque lines: any rbp spelling inside them is unknowable.
        if infos[n].pinned || infos[n].kind == LineKind::InlineAsm {
            if t.contains("%rbp")
                || t.contains("%ebp")
                || t.contains("%bp,")
                || t == "%bp"
                || t.contains("%bpl")
            {
                return false;
            }
            continue;
        }
        // Frame-pointer setup / restore / leave — BOTH width spellings.
        // A `movl %esp, %ebp` frame is exactly as much a frame as the
        // 64-bit one (three sibling passes check both; the i686 backend
        // and hand-written 32-bit assembly use the `l` forms), and
        // `enter` establishes the frame by encoding alone.
        if t.starts_with("movq %rsp, %rbp")
            || t.starts_with("mov %rsp, %rbp")
            || t.starts_with("movq %rbp, %rsp")
            || t.starts_with("mov %rbp, %rsp")
            || t.starts_with("movl %esp, %ebp")
            || t.starts_with("mov %esp, %ebp")
            || t.starts_with("movl %ebp, %esp")
            || t.starts_with("mov %ebp, %esp")
            || t.starts_with("leave")
            || t.starts_with("enter")
        {
            return false;
        }
        // CFA rooted at rbp (frame-pointer build's unwind contract).
        // Register 6 is rbp in the x86-64 CFI numbering, 5 in the i386
        // numbering — either marks a frame-pointer build.
        if (t.starts_with(".cfi_def_cfa ") && (t.contains("%rbp") || t.contains("%ebp")))
            || (t.starts_with(".cfi_def_cfa_register") && (t.ends_with(" 6") || t.ends_with(" 5")))
            || t.starts_with(".cfi_def_cfa_register %rbp")
            || t.starts_with(".cfi_def_cfa_register %ebp")
        {
            return false;
        }
        // Any parenthesised rbp operand: slot traffic or &rbp arithmetic.
        if (t.contains("(%rbp)") || t.contains("(%ebp)")) && !t.starts_with(".cfi") {
            return false;
        }
        // 16-bit/8-bit spellings inside memory operands.
        if (t.contains("(%bp)") || t.contains("(%bpl)")) && !t.starts_with(".cfi") {
            return false;
        }
    }
    true
}

/// Per-pass memo of [`rbp_is_gpr_in_function`], exact by construction.
///
/// The producer-retarget and move-relay passes ask the question once per
/// rbp-naming candidate, and each answer was a function-range scan plus a
/// linear text scan of the whole function: quadratic on the 2,000-line
/// functions FORCE_INLINE code produces (zstd `huf_compress.c`: 875 queries,
/// 2.5 G instructions).  The verdict reads only lines inside its function,
/// so it stays valid until the pass edits that function; [`Self::edited`]
/// drops exactly that function's entry and the next query re-scans the
/// current text.  A hit is therefore the predicate evaluated on the current
/// text -- the memo cannot change a decision, only skip a repeated scan.
///
/// The range comes from the caller's [`FileLiveness`] (O(1)), falling back
/// to [`function_range`] when its recorded delimiters are not intact; both
/// name the same `(start, end)` for any line inside a function.
#[derive(Default)]
pub(super) struct RbpRoles {
    verdicts: crate::common::fx_hash::FxHashMap<(usize, usize), bool>,
}

impl RbpRoles {
    fn range(
        lv: &FileLiveness,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) -> Option<(usize, usize)> {
        lv.recorded_function(store, infos, idx)
            .or_else(|| function_range(store, infos, idx))
    }

    /// [`rbp_is_gpr_in_function`] for the function containing `idx`;
    /// `false` outside a function, as at every former call site.
    pub(super) fn is_gpr(
        &mut self,
        lv: &FileLiveness,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) -> bool {
        let Some((fs, fe)) = Self::range(lv, store, infos, idx) else {
            return false;
        };
        *self
            .verdicts
            .entry((fs, fe))
            .or_insert_with(|| rbp_is_gpr_in_function(store, infos, fs, fe))
    }

    /// The pass changed line `idx` (and possibly others in the same
    /// function): forget that function's verdict.  Outside a recognisable
    /// function every verdict is dropped.
    pub(super) fn edited(
        &mut self,
        lv: &FileLiveness,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) {
        match Self::range(lv, store, infos, idx) {
            Some(key) => {
                self.verdicts.remove(&key);
            }
            None => self.verdicts.clear(),
        }
    }
}

/// Does `line` name any width of GP family `fam`? Boundary-checked so `%r1`
/// never matches inside `%r10` and `%r8` never matches inside `%r8b`.
/// Conservatively `true` for out-of-range families.
pub(super) fn line_refs_family(line: &str, fam: RegId) -> bool {
    if fam as usize >= REG_NAMES[0].len() {
        return true;
    }
    for tier in REG_NAMES.iter() {
        let name = tier[fam as usize];
        let mut start = 0;
        while let Some(pos) = line[start..].find(name) {
            let abs = start + pos;
            let end = abs + name.len();
            let boundary = line
                .as_bytes()
                .get(end)
                .is_none_or(|&c| !(c as char).is_ascii_alphanumeric());
            if boundary {
                return true;
            }
            start = end;
        }
    }
    false
}

/// `true` if `t` writes ALL of family `fam` without reading it — a `mov`-class
/// or `lea` destination whose source operand text does not mention the family.
/// `addl %r8d, %r10d` is a write AND a read, so it does not qualify.
pub(super) fn is_full_write(info: &LineInfo, t: &str, fam: RegId) -> bool {
    if get_dest_reg(info) != fam {
        return false;
    }
    let is_producer = t.starts_with("mov")
        || t.starts_with("lea")
        || t.starts_with("popcnt")
        || t.starts_with("lzcnt")
        || t.starts_with("tzcnt")
        || t.starts_with("andn")
        || t.starts_with("blsr")
        || t.starts_with("blsi")
        || t.starts_with("blsmsk")
        || t.starts_with("bzhi")
        || t.starts_with("shlx")
        || t.starts_with("shrx")
        || t.starts_with("sarx");
    if !is_producer {
        // `xorl %r10d, %r10d` (self-zeroing) is a full write as well.
        let name32 = REG_NAMES[1][fam as usize];
        let name64 = REG_NAMES[0][fam as usize];
        let self_zero = (t.starts_with("xorl ") || t.starts_with("xorq "))
            && (t.contains(&format!("{}, {}", name32, name32))
                || t.contains(&format!("{}, {}", name64, name64)));
        return self_zero;
    }
    // The source half must not read the family (`movq 8(%r13), %r13`,
    // `leaq 1(%r10), %r10`).
    let src_part = &t[..t.rfind(',').unwrap_or(t.len())];
    !line_refs_family(src_part, fam) && dest_is_full_width(t, fam)
}

/// The destination token names `fam` at 32 or 64 bits — the only widths at
/// which a write fully redefines the architectural 64-bit family (a 32-bit
/// write zero-extends).  A byte/word destination (`movb $1, %al`,
/// `movzbl %al, %al`, `movw %cx, %ax`) rewrites only part of it and must
/// never be accepted as a full redefinition: the upper bits of the old
/// value stay observable.
#[inline]
fn dest_is_full_width(t: &str, fam: RegId) -> bool {
    let dest = t.rsplit(',').next().unwrap_or(t).trim();
    dest == REG_NAMES[0][fam as usize] || dest == REG_NAMES[1][fam as usize]
}

/// Proof 1: block-local write-before-read deadness of `fam` from `from`.
pub(super) fn dead_in_block_after(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
    fam: RegId,
) -> bool {
    let mask = 1u16 << fam;
    let mut n = from;
    while n < store.len() {
        if infos[n].is_nop() {
            n += 1;
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        // Implicit readers/clobberers are invisible to the register text scan.
        if implicit_read_reg_family(t) == Some(fam) {
            return false;
        }
        if has_implicit_reg_usage(t) && fam <= 2 {
            return false; // div/mul/cltq/cqto family traffic on rax/rcx/rdx
        }
        if infos[n].is_barrier() {
            return false; // another block may read the register
        }
        if infos[n].reg_refs & mask == 0 {
            n += 1;
            continue;
        }
        if is_full_write(&infos[n], t, fam) {
            return true;
        }
        return false; // read (or read-modify-write) reaches the value
    }
    false
}

/// Half-open `[start, end)` line range of the function containing `idx`,
/// delimited by `.cfi_startproc` / `.cfi_endproc`.
///
/// Returns `None` when no `.cfi_startproc` precedes `idx`. Whole-function
/// reasoning is only valid inside a REAL function: without the delimiter the
/// text may be a fragment (a peephole unit test, a hand-written stub), and
/// "this register is never mentioned again" would then be a statement about
/// the fragment rather than about the program.
pub(super) fn function_range(
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
) -> Option<(usize, usize)> {
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
            return None; // `idx` sits between two functions
        }
    }
    let start = start?;
    let mut end = len;
    let mut n = idx;
    while n < len {
        if !infos[n].is_nop() && infos[n].trimmed(store.get(n)).starts_with(".cfi_endproc") {
            end = n;
            break;
        }
        n += 1;
    }
    Some((start, end))
}

/// The `.cfi_startproc` / `.cfi_endproc` markers of a line store, answering
/// [`function_range`] in O(log F) for passes that ask once per loop or per
/// candidate (a scan per query is quadratic in the function's length).
///
/// Exact while the caller reports every `insert_line` through
/// [`CfiIndex::shift_for_insert`] and never rewrites, nops or inserts a CFI
/// marker line itself.
pub(super) struct CfiIndex {
    /// Every live marker, ascending: `(line, is_startproc)`.
    marks: Vec<(usize, bool)>,
}

impl CfiIndex {
    pub(super) fn build(store: &LineStore, infos: &[LineInfo]) -> Self {
        let mut marks = Vec::new();
        for (n, info) in infos.iter().enumerate().take(store.len()) {
            if info.is_nop() {
                continue;
            }
            let t = info.trimmed(store.get(n));
            if t.starts_with(".cfi_startproc") {
                marks.push((n, true));
            } else if t.starts_with(".cfi_endproc") {
                marks.push((n, false));
            }
        }
        CfiIndex { marks }
    }

    /// [`function_range`]`(store, infos, idx)` for a store of `len` lines.
    pub(super) fn function_range(&self, idx: usize, len: usize) -> Option<(usize, usize)> {
        if len == 0 {
            return None;
        }
        // Nearest marker at or above the clamped index going backward: a
        // startproc opens the function, an endproc means `idx` lies
        // between two functions.
        let k = self.marks.partition_point(|&(n, _)| n <= idx.min(len - 1));
        let (start, is_start) = *self.marks.get(k.checked_sub(1)?)?;
        if !is_start {
            return None;
        }
        // First endproc at or after `idx` (startprocs are stepped over, as
        // the forward scan does); past-the-end when there is none.
        let from = self.marks.partition_point(|&(n, _)| n < idx);
        let end = self.marks[from..]
            .iter()
            .find(|&&(_, is_start)| !is_start)
            .map_or(len, |&(n, _)| n);
        Some((start, end))
    }

    /// Account for `store.insert_line(ins, ..)` of a non-CFI line.
    pub(super) fn shift_for_insert(&mut self, ins: usize) {
        for (n, _) in &mut self.marks {
            if *n >= ins {
                *n += 1;
            }
        }
    }
}

/// Registers an ABI-visible control transfer can READ without naming them in
/// the instruction text: the SysV argument registers (`%rdi %rsi %rdx %rcx
/// %r8 %r9`) and `%rax` (vector-argument count for variadic callees).
/// Whole-function textual uniqueness says nothing about those reads, so a
/// function containing a call, a tail jump or an indirect jump cannot use
/// proof 2 for these families.
///
/// `%r10` (the static chain) is deliberately NOT in this set: LCCC's
/// nested-call lowering stages the chain TEXTUALLY before every nested call
/// and every tail call to a nested callee (`leaq/movq …, %r10` — verified on
/// the nested_fn shape; the IR inserts SetStaticChain at each such call
/// site). An invisible r10 read therefore requires a textual r10 writer in
/// the same function, which `family_private_to` already treats as a
/// non-owned mention and fails on. A function whose only r10 traffic is the
/// transform's own LEA+consumer pair passes no chain and no callee can read
/// one from it. Keeping r10 in the set made every r10-scratch temp
/// unprovable across any call (the RA's most common scratch pick after
/// rax/rcx/rdx), which blocked the LEA→memory window fold in the
/// csv/rbtree loop bodies.
#[inline]
fn implicit_at_transfer(fam: RegId) -> bool {
    matches!(fam, 0 | 1 | 2 | 6 | 7 | 8 | 9)
}

/// Registers `ret` reads implicitly: the integer return value `%rax:%rdx`.
#[inline]
fn implicit_at_return(fam: RegId) -> bool {
    matches!(fam, 0 | 2)
}

/// `true` when `fam` is caller-saved under SysV (`%rax/%rcx/%rdx/%rsi/
/// %rdi/%r8-%r11`): no `ret` must-restore obligation, calls clobber it
/// anyway. Only for such families can whole-function textual uniqueness
/// justify a NEW write (Guard 1 below): a callee-saved source is owed its
/// entry value at every `ret`, an obligation no text scan can see.
#[inline]
fn is_caller_saved_family(fam: RegId) -> bool {
    matches!(fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11)
}

/// Name -> line of the FIRST live label of that name in `[start, end)`: the
/// resolution [`copy_in_loop`] and [`self_base_reexec_unsafe`] use (label text
/// carries the trailing colon, jump targets do not, mirroring
/// `FileLiveness::analyse_function`).  One pass, then O(1) per jump; the
/// linear search per jump it replaces made each query O(jumps x labels).
fn first_label_lines<'s>(
    store: &'s LineStore,
    infos: &[LineInfo],
    start: usize,
    end: usize,
) -> crate::common::fx_hash::FxHashMap<&'s str, usize> {
    let mut lines = crate::common::fx_hash::FxHashMap::default();
    for n in start..end {
        if infos[n].is_nop() || infos[n].kind != LineKind::Label {
            continue;
        }
        if let Some(name) = infos[n].trimmed(store.get(n)).strip_suffix(':') {
            lines.entry(name).or_insert(n);
        }
    }
    lines
}

/// The enclosing function of `idx`: `lv`'s record when it analysed the
/// function (O(1), the same `[start, end)` as [`function_range`]), else the
/// delimiter scan.
fn enclosing_function(
    lv: &FileLiveness,
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
) -> Option<(usize, usize)> {
    lv.recorded_function(store, infos, idx)
        .or_else(|| function_range(store, infos, idx))
}

/// `true` when line `idx` may sit inside a loop: some control edge from
/// after `idx` targets `idx` or an earlier line, so the copy re-executes.
/// Textual and deliberately conservative: an unresolvable function shape or
/// any indirect jump (`jmp *...`, e.g. a jump table or tail call) answers
/// `true`, direct jumps to labels outside the function are ignored (a tail
/// call never re-executes `idx`).
fn copy_in_loop(lv: &FileLiveness, store: &LineStore, infos: &[LineInfo], idx: usize) -> bool {
    let Some((start, end)) = enclosing_function(lv, store, infos, idx) else {
        return true;
    };
    let labels = first_label_lines(store, infos, start, end);
    let resolve = |name: &str| labels.get(name).copied();
    for q in idx + 1..end {
        match infos[q].kind {
            LineKind::Jmp | LineKind::CondJmp => {
                let t = infos[q].trimmed(store.get(q));
                let Some(target) = t.split_whitespace().nth(1) else {
                    continue;
                };
                // A misclassified indirect target fails closed (FileLiveness
                // declines the whole function on these).
                if target.starts_with('*') {
                    return true;
                }
                if let Some(pos) = resolve(target)
                    && pos <= idx
                {
                    return true;
                }
            }
            LineKind::JmpIndirect => return true,
            _ => {}
        }
    }
    false
}

/// Proof 2: inside the enclosing function, `fam` is mentioned ONLY by the
/// lines in `owned` (the ones the caller rewrites or deletes), and no
/// instruction reads the family implicitly.
pub(super) fn family_private_to(
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
    fam: RegId,
    owned: &[usize],
) -> bool {
    match function_range(store, infos, idx) {
        Some(range) => family_private_in(store, infos, range, fam, owned),
        None => false,
    }
}

/// [`family_private_to`] for a caller holding the [`FileLiveness`] of the
/// file: the enclosing function's bounds come from its record (O(1), the
/// same `[start, end)` as [`function_range`] -- see
/// `FileLiveness::recorded_function`) instead of a rescan for the
/// delimiters.  The rescan is O(function) per query, and this proof runs
/// for every candidate the dataflow calls live -- nearly all of them in
/// -O0 code, where the accumulator is live everywhere: compile time was
/// quadratic in function length.
fn family_private_lv(
    lv: &FileLiveness,
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
    fam: RegId,
    owned: &[usize],
) -> bool {
    match lv.recorded_function(store, infos, idx) {
        Some(range) => family_private_in(store, infos, range, fam, owned),
        None => family_private_to(store, infos, idx, fam, owned),
    }
}

/// The scan behind [`family_private_to`] over the function `[start, end)`.
fn family_private_in(
    store: &LineStore,
    infos: &[LineInfo],
    (start, end): (usize, usize),
    fam: RegId,
    owned: &[usize],
) -> bool {
    let mask = 1u16 << fam;
    // The proof is a conjunction over the function's lines, so any line
    // that fails it decides `false` whatever the visiting order.  Settle
    // the common failure first, on the cached bitmask alone: callers ask
    // about families the dataflow already calls live, which another
    // instruction nearly always names close to the candidate.  Walking
    // outward from the first owned line finds that mention in a few steps,
    // where the text checks below cost several string searches per line
    // over the whole function (62k queries on zstd `huf_compress.c`,
    // 1.85 G instructions).  Only a family that passes this test pays for
    // the full scan, which is unchanged.
    let names_fam =
        |n: usize| !infos[n].is_nop() && infos[n].reg_refs & mask != 0 && !owned.contains(&n);
    let pivot = owned
        .iter()
        .copied()
        .find(|&p| (start..end).contains(&p))
        .unwrap_or(start);
    if (pivot..end).any(names_fam) || (start..pivot).rev().any(names_fam) {
        return false;
    }
    let mut n = start;
    while n < end {
        if infos[n].is_nop() {
            n += 1;
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        if implicit_read_reg_family(t) == Some(fam) {
            return false;
        }
        if has_implicit_reg_usage(t) && fam <= 2 {
            return false;
        }
        // Static-chain and external-retpoline %r10 reads are the TWO
        // %r10 consumers invisible to the text scan AND to `reg_refs`
        // (the marker is a comment; the thunk name encodes no `%r10`
        // operand). They are exactly the reads whose staging writes the
        // transforms in this module want to delete: `owned` may cover
        // every textual %r10 mention (the staging itself plus the
        // rewritten consumer) and still leave an invisible live reader.
        // Mirror FileLiveness's protocol EXACTLY -- the shared marker-block
        // parser (`liveness::call_markers`), and every
        // `call __x86_indirect_thunk_*` reading the r10-staged target --
        // so the two liveness oracles can never disagree about a chain
        // call (a divergent pair is itself a miscompile-in-waiting).
        if fam == 10 {
            if t.starts_with("call __x86_indirect_thunk_") {
                return false;
            }
            if matches!(infos[n].kind, LineKind::Call)
                && super::liveness::call_markers(store, infos, n).chain
            {
                return false;
            }
        }
        // ABI-implicit reads at control transfers are invisible to the text
        // scan: `call foo` reads %rdi..%r9/%rax, `ret` reads %rax:%rdx.
        // (Static-chain and external-retpoline %r10 reads are handled by
        // the fam-10 case above -- the marker/thunk evidence, not textual
        // traffic. See implicit_at_transfer's doc.)
        // A family the caller keeps live across this call under the
        // interprocedural allocation contract (`# LCCC_IPA_KEEP`) carries a
        // value through the call: never private to the owned lines.
        if matches!(infos[n].kind, LineKind::Call)
            && super::liveness::call_markers(store, infos, n).ipa_keep & mask != 0
        {
            return false;
        }
        match infos[n].kind {
            LineKind::Call | LineKind::JmpIndirect if implicit_at_transfer(fam) => return false,
            // A jump to a non-local target is a tail call and reads the
            // argument registers; `.L*` targets are intra-function.
            LineKind::Jmp
                if implicit_at_transfer(fam)
                    && !t
                        .trim_start_matches(|c: char| c != ' ')
                        .trim()
                        .starts_with('.') =>
            {
                return false;
            }
            LineKind::Ret if implicit_at_return(fam) => return false,
            _ => {}
        }
        if infos[n].reg_refs & mask == 0 {
            n += 1;
            continue;
        }
        if !owned.contains(&n) {
            return false;
        }
        n += 1;
    }
    true
}

/// Combined deadness: either proof suffices (see the module header).
pub(super) fn provably_dead(
    store: &LineStore,
    infos: &[LineInfo],
    use_idx: usize,
    fam: RegId,
    owned: &[usize],
) -> bool {
    dead_in_block_after(store, infos, use_idx + 1, fam)
        || family_private_to(store, infos, use_idx, fam, owned)
}

/// Deadness with the exact analysis first: [`FileLiveness`] answers precisely
/// whenever the enclosing function is analysable, and the two syntactic proofs
/// remain as a fallback for functions it declines (indirect jumps, unknown
/// mnemonics, missing CFI).
pub(super) fn provably_dead_lv(
    lv: &FileLiveness,
    store: &LineStore,
    infos: &[LineInfo],
    use_idx: usize,
    fam: RegId,
    owned: &[usize],
) -> bool {
    // The three proofs are independent and each is sound on its own, so the
    // union is used. The dataflow answer is not an authority that can veto the
    // others: it models a sub-register write (`sete %r10b`) as also READING the
    // family, which keeps a boolean's upper bits "live" around a loop even
    // though the next instruction (`movzbl %r10b, %r10d`) kills them — exactly
    // the case whole-function uniqueness settles.
    if matches!(lv.live_after(use_idx, fam), Some(false)) {
        return true;
    }
    dead_in_block_after(store, infos, use_idx + 1, fam)
        || family_private_lv(lv, store, infos, use_idx, fam, owned)
}

/// Deadness oracle for passes that delete a definition and rewrite its
/// consumer: exact CFG liveness ([`FileLiveness`]) built on first use and
/// refreshed per dirtied function on demand, with [`provably_dead`]'s
/// syntactic proofs as the fallback for functions the dataflow declines.
///
/// It replaces a forward TEXT scan ("no read of the family on any later
/// line of the function") that the LEA/SIB folds used to rely on.  That scan
/// ignores control flow in the backward direction: a read at the top of a
/// loop is reached around the back edge from a definition at the bottom, yet
/// sits EARLIER in the text.  `leaq 8(%rsi), %r9; .L: movq (%r9), %rax; ...;
/// leaq 8(%r13, %r10), %r9; movq (%r9), %rcx; jmp .L` had its second LEA
/// folded away, so the next iteration dereferenced the stale `%r9`.
pub(super) struct LazyDeadness {
    lv: Option<FileLiveness>,
    /// First line of every function rewritten since the last refresh.
    dirty: Vec<usize>,
}

impl LazyDeadness {
    pub(super) fn new() -> Self {
        LazyDeadness {
            lv: None,
            dirty: Vec::new(),
        }
    }

    /// Is `fam` dead after line `use_idx`?  `owned` lists the lines the
    /// caller is about to rewrite or delete (the only mentions the
    /// whole-function fallback may ignore).
    pub(super) fn dead_after(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        use_idx: usize,
        fam: RegId,
        owned: &[usize],
    ) -> bool {
        if fam > REG_GP_MAX {
            return false;
        }
        let lv = self
            .lv
            .get_or_insert_with(|| FileLiveness::new(store, infos));
        for start in self.dirty.drain(..) {
            lv.refresh_at(store, infos, start);
        }
        provably_dead_lv(lv, store, infos, use_idx, fam, owned)
    }

    /// Record that line `idx` (and so its function) was rewritten.  A fold
    /// extends the live ranges of the registers it substitutes in, so cached
    /// answers for that function are stale until the next query refreshes it.
    pub(super) fn invalidate(&mut self, store: &LineStore, infos: &[LineInfo], idx: usize) {
        let Some(lv) = self.lv.as_ref() else {
            return;
        };
        let start = Self::function_start(lv, store, infos, idx);
        if !self.dirty.contains(&start) {
            self.dirty.push(start);
        }
    }

    /// Record that the caller edited ONLY lines in `lo..=hi` (one function;
    /// the lines between its edits unchanged), and bring the cached solution
    /// up to date now with the exact local splice
    /// ([`FileLiveness::refresh_span`]) instead of deferring a whole-function
    /// re-analysis to the next query.
    ///
    /// The splice must run before any further edit: it reads the
    /// pre-edit solution for everything outside the span, so it cannot be
    /// deferred and batched like [`Self::invalidate`].  A function already
    /// queued by `invalidate` is left to that full re-analysis, which covers
    /// this edit as well.
    ///
    /// `fold_lea_into_memory_op` re-solved the whole function after each of
    /// its ~1,000 folds on zstd `huf_compress.c` (2.84 G instructions, 8% of
    /// the compile); each fold rewrites two or three adjacent lines.
    pub(super) fn invalidate_span(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        lo: usize,
        hi: usize,
    ) {
        let Some(lv) = self.lv.as_mut() else {
            return;
        };
        if !self.dirty.is_empty()
            && self
                .dirty
                .contains(&Self::function_start(lv, store, infos, lo))
        {
            return;
        }
        lv.refresh_span(store, infos, lo, hi);
    }

    /// The `.cfi_startproc` line of `idx`'s function (`idx` itself outside
    /// one): the solution's recorded bounds in O(1), else the text scan --
    /// both name the same line.
    fn function_start(
        lv: &FileLiveness,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
    ) -> usize {
        lv.recorded_function(store, infos, idx)
            .or_else(|| function_range(store, infos, idx))
            .map_or(idx, |(s, _)| s)
    }
}

/// Split `OP SRC, DST` (AT&T, dest last) into the trimmed operand texts.
pub(super) fn split_two_operands(rest: &str) -> Option<(&str, &str)> {
    // AT&T SIB operands contain internal commas: `src, disp(%base,%idx,4)`.
    // Splitting at the raw last comma turns `%idx)` into a fake destination
    // register.  Dead-pure-write elimination then classifies an actual memory
    // store as a register-only move and deletes it. Use the shared balanced-
    // parentheses scanner that line classification already relies on.
    let comma = last_top_level_comma(rest.as_bytes())?;
    Some((rest[..comma].trim(), rest[comma + 1..].trim()))
}

/// Parse a bare register operand into its GP family, rejecting anything that
/// is not a plain `%reg` (memory operands, immediates, XMM/MMX registers).
pub(super) fn plain_gp_operand(text: &str) -> Option<RegId> {
    if !text.starts_with('%') || text.contains('(') {
        return None;
    }
    let fam = register_family_fast(text);
    if fam == REG_NONE || fam > REG_GP_MAX {
        return None;
    }
    Some(fam)
}

// ── Pass 1: move-relay elimination ───────────────────────────────────────────

/// The width in bits of a plain GP register spelling. Unlike `dest_width`
/// (whose 64/32-only contract two callers rely on) this covers 8/16-bit
/// spellings too; high-byte spellings (`%ah`) report 8, so callers needing
/// canonical-low must compare spellings (see `relay_source_for`).
fn operand_width(name: &str) -> Option<u8> {
    let fam = register_family_fast(name);
    if fam == REG_NONE || fam > REG_GP_MAX {
        return None;
    }
    let f = fam as usize;
    if name == REG_NAMES[0][f] {
        Some(64)
    } else if name == REG_NAMES[1][f] {
        Some(32)
    } else if name == REG_NAMES[2][f] {
        Some(16)
    } else if name == REG_NAMES[3][f] {
        Some(8)
    } else if matches!(name, "%ah" | "%ch" | "%dh" | "%bh") {
        Some(8)
    } else {
        None
    }
}

/// P4's width-flexible relay source: `Some` respelled `%S` text when the
/// consumer's source operand reads `%D` in a relayable way. The family must
/// match at a canonical low spelling — high-byte uses (`%ah`) read bits
/// 8-15, which no copy establishes — and the width rule decides the proof:
/// reads at or below the copy width are free (the copy preserves the low
/// bits, so `movq %S, %D` + `cmpl %D32, %X` ≡ `cmpl %S32, %X` with no proof
/// at all); a 64-bit read under a 32-bit copy needs `upper32_zero_at` (the
/// upper half is otherwise unknown). The rewritten line reads identical
/// values (flags included — carry flows through the same line unchanged).
fn relay_source_for(
    use_src: &str,
    dst_fam: RegId,
    src_fam: RegId,
    copy_w: u8,
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
) -> Option<String> {
    if plain_gp_operand(use_src) != Some(dst_fam) {
        return None;
    }
    let read_w = operand_width(use_src)?;
    let tier = match read_w {
        64 => 0,
        32 => 1,
        16 => 2,
        8 => 3,
        _ => return None,
    };
    if use_src != REG_NAMES[tier][dst_fam as usize] {
        return None;
    }
    if read_w > copy_w && !upper32_zero_at(store, infos, i, src_fam) {
        return None;
    }
    Some(REG_NAMES[tier][src_fam as usize].to_string())
}

/// Delete `mov %S, %D` when the copy exists only to feed one ALU source:
///
/// ```text
///     movl %eax, %r10d          movzbl (%rsi,%rbx), %eax
///     addl %r10d, %r8d     ->   addl %eax, %r8d
/// ```
///
/// Conditions (all checked):
/// * `%S` and `%D` are distinct general-purpose registers (never `%rsp`;
///   `%rbp` only where [`rbp_is_gpr_in_function`] proves it allocator-owned,
///   and never renamed into a memory operand), and the copy is a plain
///   register-to-register `movl`/`movq`.
/// * Between the copy and the use there is no barrier, no implicit register
///   traffic, no write to `%S`, and no other mention of `%D`.
/// * The use names `%D` EXACTLY as the copy's destination text, so the rewrite
///   is width-flexible: reads at or below the copy width relay free (the
///   copy preserves the low bits); a 64-bit read under a 32-bit copy needs
///   `upper32_zero_at` (high-byte uses never relay).
/// * The use's destination is a different family, and after substituting `%S`
///   the line no longer mentions `%D` at all.
/// * `%D` is provably dead after the use (module header, proofs 1 and 2).

pub(super) fn eliminate_move_relays(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut rbp_roles = RbpRoles::default();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let mov = infos[i].trimmed(store.get(i));
        let Some(rest) = mov
            .strip_prefix("movq ")
            .or_else(|| mov.strip_prefix("movl "))
        else {
            i += 1;
            continue;
        };
        let Some((src_text, dst_text)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(src_fam), Some(dst_fam)) =
            (plain_gp_operand(src_text), plain_gp_operand(dst_text))
        else {
            i += 1;
            continue;
        };
        // `%rbp` relays only where whole-function evidence proves it is an
        // allocator-owned GPR (`rbp_is_gpr_in_function`, the same gate the
        // producer-retarget and flag passes use). Refusing it outright left
        // the relay alive whenever the allocator happened to home the source
        // in rbp: zstd_count's `movl %ebp, %r14d; leaq buffer(%rip), %r10;
        // addq %r14, %r10` kept a zero-extension copy per iteration that the
        // identical `%r12` shape folds. The range scan runs lazily, so copies
        // that do not name rbp never pay for it.
        let rbp_gpr =
            (src_fam == REG_RBP || dst_fam == REG_RBP) && rbp_roles.is_gpr(&lv, store, infos, i);
        if src_fam == dst_fam
            || !is_writable_family_gpr(src_fam, rbp_gpr)
            || !is_writable_family_gpr(dst_fam, rbp_gpr)
        {
            i += 1;
            continue;
        }
        let copy_w = dest_width(dst_text).unwrap_or(0);
        let dst_mask = 1u16 << dst_fam;

        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() || infos[j].pinned {
                break;
            }
            let t = infos[j].trimmed(store.get(j)).to_string();
            let t = t.as_str();
            if has_implicit_reg_usage(t) {
                break; // div/mul/string ops: unmodelled register traffic
            }
            if infos[j].reg_refs & dst_mask != 0 {
                // This is the first line touching %D — it must be the relay
                // consumer, or the transform is off.
                let mut folded = false;
                // A store whose SOURCE is the copy: `movl %D, MEM` becomes
                // `movl %S, MEM`. Same conditions as the ALU case — the
                // operand text must match exactly (width-exact) and %D must be
                // dead afterwards.
                if !folded {
                    if let Some(sop) = ["movq ", "movl ", "movw ", "movb "]
                        .iter()
                        .find(|op| t.starts_with(**op))
                    {
                        if let Some((st_src, st_dst)) = split_two_operands(&t[sop.len()..]) {
                            if st_dst.contains('(') {
                                let Some(new_src) = relay_source_for(
                                    st_src, dst_fam, src_fam, copy_w, store, infos, i,
                                ) else {
                                    break;
                                };
                                let new_line = format!("    {sop}{new_src}, {st_dst}");
                                if !line_refs_family(&new_line, dst_fam)
                                    && provably_dead_lv(&lv, store, infos, j, dst_fam, &[i, j])
                                {
                                    mark_nop(&mut infos[i]);
                                    replace_line(store, &mut infos[j], j, new_line);
                                    // Only `i` (now a nop) and `j` changed.
                                    lv.refresh_span(store, infos, i, j);
                                    rbp_roles.edited(&lv, store, infos, i);
                                    changed = true;
                                    folded = true;
                                }
                            }
                        }
                    }
                }
                if !folded {
                    if let Some(op) = RELAY_OPS.iter().find(|op| t.starts_with(**op)) {
                        if let Some((use_src, use_dst)) = split_two_operands(&t[op.len()..]) {
                            let use_dst_fam = register_family_fast(use_dst);
                            let dst_is_other_reg = use_dst.starts_with('%')
                                && use_dst_fam != REG_NONE
                                && use_dst_fam != dst_fam;
                            if dst_is_other_reg {
                                let Some(new_src) = relay_source_for(
                                    use_src, dst_fam, src_fam, copy_w, store, infos, i,
                                ) else {
                                    break;
                                };
                                let new_line = format!("    {op}{new_src}, {use_dst}");
                                // The substitution must absorb EVERY mention of %D.
                                if !line_refs_family(&new_line, dst_fam)
                                    && provably_dead_lv(&lv, store, infos, j, dst_fam, &[i, j])
                                {
                                    mark_nop(&mut infos[i]);
                                    replace_line(store, &mut infos[j], j, new_line);
                                    // Only `i` (now a nop) and `j` changed.
                                    lv.refresh_span(store, infos, i, j);
                                    rbp_roles.edited(&lv, store, infos, i);
                                    changed = true;
                                    folded = true;
                                }
                            }
                        }
                    }
                }
                if !folded {
                    // The copy's destination is mentioned inside a MEMORY
                    // operand (`addq $1, (%r9, %r11, 8)`, `movzbl (%rdi, %r9),
                    // %edx`): the SIB base/index registers are read before the
                    // instruction's own register writes, so renaming them to
                    // the copy's source is value-preserving even when the
                    // consumer later overwrites that source (`movzbl (%rdi,
                    // %rdx), %edx` — the address is computed first) and even
                    // when the source is mentioned elsewhere in the line: the
                    // relay invariant gives %D == %S at the consumer, so all
                    // mentions fold to one value (`addq $1, (%r11, %r9, 8)`
                    // with %r11 a copy of %r9 is `(%r9, %r9, 8)`). Memory
                    // operands read their registers at 64 bits, so a 32-bit
                    // copy needs `upper32_zero_at` exactly like any other
                    // 64-bit read of the source.
                    //
                    // The one hazard is the rename introducing a REGISTER
                    // WRITE of %S: `imull %esi, (%r9,%r11,8), %r11d` folded to
                    // `%eax` would zero-extend over %rax's live high half. A
                    // bare-register destination naming %D therefore refuses —
                    // unless that destination is a write-only full-width def
                    // of %D (`fam_written_only_as_dest`): then the source-only
                    // rename leaves the destination register untouched (it
                    // holds a FRESH value, not the copy's), no register write
                    // of %S appears anywhere, and the copy's value is killed
                    // at the consumer by the destination write itself — so no
                    // later-deadness proof is needed or wanted (the new
                    // destination value may well be live). A memory
                    // destination's registers are reads, never the write.
                    let writes_dst_as_reg = consumer_writes_dst_as_reg(t, dst_fam);
                    let write_only_dst = writes_dst_as_reg && fam_written_only_as_dest(t, dst_fam);
                    //
                    // Never rename INTO a memory operand as `%rbp`: every
                    // `N(%rbp)` spelling is parsed as frame-slot traffic
                    // (`LineInfo::rbp_offset`, store forwarding, dead-store
                    // elimination) and is itself the frame-pointer evidence
                    // `rbp_is_gpr_in_function` keys on — the rewrite would
                    // invalidate the very proof that admitted it.
                    if src_fam != REG_RBP
                        && t.contains('(')
                        && (!writes_dst_as_reg || write_only_dst)
                    {
                        // The high-byte trap (pinned by
                        // `test_rcx_address_copy_high_byte_use_is_not_a_kill`):
                        // `%ch` reads bits 8..15 of the copied value at a
                        // LATER line, and no canonical spelling of the family
                        // names that read. Established remedy, as in
                        // `copy_coalesce` rule 5: veto when a high-byte
                        // spelling of either family appears anywhere in the
                        // enclosing function.
                        let high_byte_ok = {
                            let hb: [(&str, RegId); 4] =
                                [("%ah", 0), ("%ch", 1), ("%dh", 2), ("%bh", 3)];
                            let (lo, hi) =
                                function_range(store, infos, i).unwrap_or((0, store.len()));
                            (lo..hi).all(|k| {
                                infos[k].is_nop() || {
                                    let tk = infos[k].trimmed(store.get(k));
                                    !hb.iter().any(|(name, fam)| {
                                        (*fam == src_fam || *fam == dst_fam) && tk.contains(name)
                                    })
                                }
                            })
                        };
                        // A memory operand reads its registers at 64 bits —
                        // except under a 32-bit LEA: `leal` keeps only the
                        // low 32 bits of base + index*scale + disp, and those
                        // depend only on the low 32 bits of base and index
                        // (addition and scaling by 1/2/4/8 carry upward
                        // only). So a `movl` copy relays into `leal` with no
                        // upper-half proof: `movl %eax,%r11d; leal (%r11,
                        // %r11,2),%eax` ≡ `leal (%rax,%rax,2),%eax`.
                        let reads_64 = !t.starts_with("leal ");
                        if high_byte_ok
                            && (!reads_64
                                || copy_w == 64
                                || upper32_zero_at(store, infos, i, src_fam))
                        {
                            let new_line = if write_only_dst {
                                replace_reg_family_in_source(t, dst_fam, src_fam)
                                    .map(|s| format!("    {}", s))
                            } else {
                                Some(format!("    {}", replace_reg_family(t, dst_fam, src_fam)))
                            };
                            if let Some(new_line) = new_line {
                                let ok = if write_only_dst {
                                    // The destination still names %D — that is
                                    // the point; the source-only rename proved
                                    // nothing else does.
                                    fam_written_only_as_dest(&new_line, dst_fam)
                                } else {
                                    !line_refs_family(&new_line, dst_fam)
                                        && provably_dead_lv(&lv, store, infos, j, dst_fam, &[i, j])
                                };
                                if ok {
                                    mark_nop(&mut infos[i]);
                                    replace_line(store, &mut infos[j], j, new_line);
                                    // Only `i` (now a nop) and `j` changed.
                                    lv.refresh_span(store, infos, i, j);
                                    rbp_roles.edited(&lv, store, infos, i);
                                    changed = true;
                                    folded = true;
                                }
                            }
                        }
                    }
                }
                let _ = folded;
                break;
            }
            // %S redefined (explicitly or implicitly — `cqto` overwrites
            // %rdx without naming it) before the use: the copy is not a
            // relay.
            if writes_family(&infos[j], infos[j].trimmed(store.get(j)), src_fam) {
                break;
            }
            j += 1;
        }
        i += 1;
    }
    changed
}

/// `true` when `t`'s destination operand is a BARE REGISTER naming `fam` —
/// the line writes that register (possibly reading it first), so a relay
/// rename of `fam` would move the write onto the copy's source register and
/// clobber its live high bits (a 32-bit RMW like `addl $1, %r11d` zero-
/// extends). A memory destination's registers are address READS, never the
/// written value, so SIB mentions never count. Three-operand forms name the
/// destination last (`imul %esi, (%r9,%r11,8), %r11d`); one-operand RMW
/// (`incq %r11`) names it only.
fn consumer_writes_dst_as_reg(t: &str, fam: RegId) -> bool {
    let trimmed = t.trim_start();
    let Some(sp) = trimmed.find(' ') else {
        let dst = trimmed;
        return dst.starts_with('%') && !dst.contains('(') && line_refs_family(dst, fam);
    };
    let ops = trimmed[sp + 1..].trim_start();
    let dst = match last_top_level_comma(ops.as_bytes()) {
        Some(c) => ops[c + 1..].trim(),
        None => ops.trim(),
    };
    dst.starts_with('%') && !dst.contains('(') && line_refs_family(dst, fam)
}

/// `true` when `fam` is written by `t` ONLY as a bare-register destination
/// that the instruction overwrites without reading (`imull %esi, (%r9,%r11,8),
/// %r11d` — the three-operand imul's destination is a fresh value). Every
/// other register-operand mention of `fam` disqualifies: a source read would
/// want the copy's value at a spot the source-only rename does not retarget,
/// and an RMW destination (`addq (%r9,%r11,8), %r11`) reads the copy's value
/// from a register the fold would leave undefined. Mentions inside MEMORY
/// operands are address reads — exactly what the relay rename retargets — and
/// do not count against this predicate. The destination must be full-width
/// (`%r11`/`%r11d`; a 32-bit write zero-extends into the full family), so
/// the write kills the copy's value here and no later read can want it.
pub(super) fn fam_written_only_as_dest(t: &str, fam: RegId) -> bool {
    // A LEA never reads its destination as a register: its only source is
    // the address, whose register mentions are exactly what the relay
    // rename retargets (and what the memory-stripping check below ignores).
    // `is_read_modify_write`'s LEA clause is an exact-SPELLING self-address
    // test (`leaq (%r11,%r11,2), %r11` → RMW, the 32-bit twin `leal (%r11,
    // %r11,2), %r11d` → not), so consulting it here refused the 64-bit form
    // of a fold it admitted at 32 bits (`usel(a) * 3` kept its relay copy).
    let is_lea = t.trim_start().starts_with("lea");
    if !consumer_writes_dst_as_reg(t, fam) || (!is_lea && is_read_modify_write(t.trim_start())) {
        return false;
    }
    let trimmed = t.trim_start();
    let Some(sp) = trimmed.find(' ') else {
        return false;
    };
    let ops = trimmed[sp + 1..].trim_start();
    let Some(c) = last_top_level_comma(ops.as_bytes()) else {
        return false;
    };
    let (src_part, dst_part) = (ops[..c].trim(), ops[c + 1..].trim());
    if !line_refs_family(dst_part, fam) {
        return false;
    }
    if dest_width(dst_part) != Some(64) && dest_width(dst_part) != Some(32) {
        return false;
    }
    // Any register mention of `fam` outside memory operands must be the
    // destination alone; strip balanced parentheses and inspect the rest.
    let mut no_mem = String::with_capacity(src_part.len());
    let mut depth = 0usize;
    for ch in src_part.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => no_mem.push(ch),
            _ => {}
        }
    }
    !line_refs_family(&no_mem, fam)
}

/// Rename `old_id`→`new_id` family spellings in the SOURCE operands only
/// (everything before the last top-level comma); the destination operand is
/// emitted verbatim. Used by the relay fold when the consumer writes the
/// copy's destination as a fresh register value ([`fam_written_only_as_dest`]):
/// the memory-operand mentions retarget to the copy's source while the
/// destination keeps its own register. The split at the last comma is
/// top-level-exact precisely because that variant only runs for bare-register
/// destinations — a destination carries no comma, so the last comma in the
/// line is always its separator (never one inside a SIB list).
fn replace_reg_family_in_source(t: &str, old_id: RegId, new_id: RegId) -> Option<String> {
    let c = last_top_level_comma(t.as_bytes())?;
    let (src_part, dst_part) = (&t[..c], &t[c..]);
    Some(format!(
        "{}{}",
        replace_reg_family(src_part, old_id, new_id),
        dst_part
    ))
}

// ── Pass 2: windowed LEA → memory-operand folding ────────────────────────────

/// Parse `leaq ADDR, %T` and return `(addr_text, dst_text, register families
/// the address reads)`.
///
/// Accepted address forms: `DISP(%base)`, `(%base,%index)` and
/// `DISP(%base,%index,scale)`, plus the base-less `DISP(,%index,scale)` the
/// scaled-lea peephole emits and the symbolic `sym(%rip)` form. `sym(%rip)`
/// reads no register, so it is reproducible at any later use and folds into
/// a bare `(%T)` memory operand; it must never be combined with an index
/// (x86-64 `%rip` addressing has no SIB).
fn parse_lea_address(lea: &str) -> Option<(&str, &str, Vec<RegId>)> {
    let rest = lea.strip_prefix("leaq ")?;
    let (addr, dst) = rest.rsplit_once(',')?;
    let (addr, dst) = (addr.trim(), dst.trim());
    // `rsplit_once(',')` split inside the SIB list when a scale is present;
    // detect that by an unbalanced parenthesis and re-split at the real end.
    if addr.matches('(').count() != addr.matches(')').count() {
        return None;
    }
    let open = addr.find('(')?;
    let close = addr.rfind(')')?;
    if close + 1 != addr.len() || close <= open {
        return None;
    }
    let disp = addr[..open].trim();
    let mut fams = Vec::new();
    let fields: Vec<&str> = addr[open + 1..close].split(',').map(str::trim).collect();
    if fields.is_empty() || fields.len() > 3 {
        return None;
    }
    // RIP-relative symbolic displacement: `leaq sym(%rip), %T` computes the
    // address of a local symbol; no register is involved, so the fold is a
    // pure operand rewrite and the address is reproducible at any later use.
    // Accept `sym(%rip)` (symbolic or numeric displacement); rip takes no
    // slot in `fams` (it is not a family).
    let rip_base = fields.len() > 0 && fields[0] == "%rip";
    if rip_base && fields.len() == 1 {
        if disp.contains(',')
            || disp.contains('(')
            || disp.contains(' ')
            || !disp
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".$_+-@".contains(c))
        {
            return None;
        }
        return Some((addr, dst, fams));
    }
    if !rip_base && !disp.is_empty() && disp.parse::<i64>().is_err() {
        return None; // symbolic displacement (register-based) unsupported
    }
    for (n, f) in fields.iter().enumerate() {
        if n == 2 {
            if !matches!(*f, "1" | "2" | "4" | "8") {
                return None;
            }
            continue;
        }
        if f.is_empty() && n == 0 && fields.len() == 3 {
            continue; // base-less `DISP(,%idx,scale)`
        }
        if *f == "%rip" {
            return None;
        }
        let fam = register_family_fast(f);
        if fam == REG_NONE || fam > REG_GP_MAX {
            return None;
        }
        fams.push(fam);
    }
    if fams.is_empty() {
        return None;
    }
    Some((addr, dst, fams))
}

/// Fold `leaq DISP(%base), %T` into a later memory operand `(%T)` inside the
/// same basic block:
///
/// ```text
///     leaq   1(%rbx), %r10          movzbl (%rbx), %r13d
///     movzbl (%rbx), %r13d     ->   ...
///     ...                           movzbl 1(%rbx), %r14d
///     movzbl (%r10), %r14d
/// ```
///
/// `fold_lea_into_memory_op` handles the adjacent case; this pass searches a
/// bounded window and accepts the block-local deadness proof, which is what
/// unrolled byte loops (adler32) need — there the temporary is recycled by the
/// next LEA a few instructions later, so the whole-function `fam_read_after`
/// scan always reports it live.
///
/// Is re-executing the self-base LEA at `idx` UNSAFE for the fold — i.e. can
/// control return to (or before) `idx` while `fam` still carries a value that
/// flowed around the back edge?
///
/// Sound answer via the dataflow oracle: for every back edge that can
/// re-execute `idx` (a jump after `idx` targeting a label at or before it),
/// the target label's block-entry liveness of `fam` must be DEAD. Dead at
/// entry means every path from the landing point redefines `fam` before any
/// read — and the LEA itself reads `fam` — so the LEA's input always comes
/// from a fresh redefinition, never from the previous iteration's leftover.
/// An unknown (declined function, stale after a refresh) or live answer is
/// conservatively unsafe.
fn self_base_reexec_unsafe(
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
    fam: RegId,
    lv: &FileLiveness,
) -> bool {
    let Some((start, end)) = enclosing_function(lv, store, infos, idx) else {
        return true;
    };
    // Label resolution mirrors copy_in_loop (first live label of the name).
    let labels = first_label_lines(store, infos, start, end);
    if labels.is_empty() {
        return false;
    }
    let resolve = |name: &str| labels.get(name).copied();
    for q in idx + 1..end {
        match infos[q].kind {
            LineKind::Jmp | LineKind::CondJmp => {
                let t = infos[q].trimmed(store.get(q));
                let Some(target) = t.split_whitespace().nth(1) else {
                    continue;
                };
                // An unresolvable indirect target fails closed, exactly as
                // copy_in_loop does.
                if target.starts_with('*') {
                    return true;
                }
                if let Some(pos) = resolve(target)
                    && pos <= idx
                    && lv.live_after(pos, fam) != Some(false)
                {
                    return true;
                }
            }
            LineKind::JmpIndirect => return true,
            _ => {}
        }
    }
    false
}

/// One x86-64 memory operand, split into the parts the SIB byte and the
/// ModRM displacement can actually carry: an optional symbolic displacement,
/// a numeric displacement, an optional base register (always scale 1) and an
/// optional index register with a scale of 1, 2, 4 or 8.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SibAddr {
    /// Symbolic part, e.g. `table` in `table+8(%rax)`.  Two symbols can never
    /// be added, so composition rejects a pair of them.
    sym: Option<String>,
    /// Numeric displacement added to `sym` when one is present.
    disp: i64,
    base: Option<String>,
    index: Option<(String, u8)>,
}

/// Parse the displacement text of an operand: `""`, `8`, `-8`, `table`,
/// `table+8`, `table-8`.  Returns `None` for anything else so callers fail
/// closed rather than invent an address.
fn parse_disp(text: &str) -> Option<(Option<String>, i64)> {
    let text = text.trim();
    if text.is_empty() {
        return Some((None, 0));
    }
    if let Ok(v) = text.parse::<i64>() {
        return Some((None, v));
    }
    // Hex displacements: the x86 emitter formats `$imm` operands in hex, and
    // although it does not currently do so for the displacement field, a
    // numeric literal arriving here must not be mistaken for a SYMBOL.  As a
    // symbol it would still render (`0x10+8(%rax)` evaluates correctly in
    // GAS), but the displacement arithmetic in `compose_sib` would be skipped
    // and two such operands could not be summed -- a silent loss of folding
    // rather than wrong code, and cheap to rule out.
    if let Some(d) = text
        .strip_prefix("-0x")
        .or_else(|| text.strip_prefix("-0X"))
    {
        // Negated hex is parsed as a magnitude and negated, so `-0x8000000000000000`
        // fails closed instead of wrapping to the same value as the positive form.
        return i64::from_str_radix(d, 16)
            .ok()
            .and_then(|v| v.checked_neg())
            .map(|v| (None, v));
    }
    if let Some(d) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        // UNSIGNED parse, reinterpreted as a bit pattern: the emitter's 64-bit
        // displacements include values above `i64::MAX`, and `i64::from_str_radix`
        // would reject `0xffffffffffffffff` outright.  A value too wide for u64
        // is not a displacement at all, so it fails closed.
        return u64::from_str_radix(d, 16).ok().map(|v| (None, v as i64));
    }
    if text == "-" || text == "+" {
        // A bare sign is not a displacement and must not become a symbol.
        return None;
    }
    // `sym[+|-]N`: split at the LAST sign so `a-b-4` can never be reached
    // through a malformed input, and require a non-empty symbol.
    let bytes = text.as_bytes();
    for i in (1..text.len()).rev() {
        if bytes[i] == b'+' || bytes[i] == b'-' {
            let sym = &text[..i];
            if sym.is_empty() {
                return None;
            }
            let off = text[i..].parse::<i64>().ok()?;
            if !is_plain_symbol(sym) {
                return None;
            }
            return Some((Some(sym.to_string()), off));
        }
    }
    if !is_plain_symbol(text) {
        return None;
    }
    Some((Some(text.to_string()), 0))
}

/// A displacement symbol the composition may carry verbatim: an assembler
/// identifier (`table`, `.LC0`, `x.1`, `sym@PLT`-style specifiers) and
/// nothing else.  Anything with an operand prefix (`%fs:`, `*`), whitespace
/// (a mnemonic or `lock` prefix picked up by `operand_start`) or punctuation
/// is not a displacement: treating it as one would print it back glued to
/// the composed operand.
fn is_plain_symbol(sym: &str) -> bool {
    let mut chars = sym.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '.')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '@'))
}

/// Whether repeated materializations denote one stable relocatable address.
/// Numeric RIP offsets, local numeric-label references and the assembler's
/// current-location counter are position-dependent, even with identical text.
pub(super) fn is_stable_symbol_expr(text: &str) -> bool {
    matches!(parse_disp(text), Some((Some(symbol), _)) if symbol != ".")
}

/// Byte length of a leading segment-override prefix (`%gs:`, `%fs:`, `%es:`,
/// `%ds:`, `%cs:`, `%ss:`) in operand text, if one is present.
///
/// AT&T spells a segment override as `%<seg>:` between the operand boundary
/// and the displacement.  `parse_disp` deliberately refuses that spelling —
/// `is_plain_symbol` would otherwise print `%gs:` back out as a displacement
/// symbol — so every composition path used to fail closed on
/// `movq %gs:(%r9), %rax`.  That also refused the fold GCC and Clang always
/// perform (`leaq 8(%rax), %rcx; movq %gs:(%rcx), %rax` ->
/// `movq %gs:8(%rax), %rax`: one instruction and one GPR less per per-CPU
/// access, which is what every `this_cpu_ptr(p)->field` in the kernel pays).
/// Callers strip the validated prefix, compose the address behind it, and
/// re-attach the prefix verbatim; anything that is not exactly one of the six
/// segment registers, or that carries trailing text inside the prefix, stays
/// refused.
///
/// This is the *authoritative* segment-override validator for the peephole
/// layer: every pass that is willing to carry a `%gs:` operand through a
/// rewrite calls either this or [`strip_segment_prefix`], so the accepted
/// spelling is defined in exactly one place.
pub(super) fn segment_prefix_len(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    if b.len() < 4 || b[0] != b'%' || b[3] != b':' {
        return None;
    }
    // Case-insensitive: the integrated assembler accepts `%GS:` too, and the
    // prefix must be exactly two letters — `%gsx:` or `%g:` is not a prefix.
    let seg = (b[1] | 0x20, b[2] | 0x20);
    matches!(
        seg,
        (b'c', b's') | (b'd', b's') | (b'e', b's') | (b'f', b's') | (b'g', b's') | (b's', b's')
    )
    .then_some(4)
}

/// Split a validated segment override off `op`: `("%gs:", "8(%rsi)")`, or
/// `("", op)` when there is none.  The pair form of [`segment_prefix_len`]
/// for the call sites whose rewrite carries the operand verbatim and only
/// needs to judge the address *body* (`(disp(base[,index,scale]))`) — the
/// prefix itself is re-emitted untouched, so validating it is all it takes.
pub(super) fn strip_segment_prefix(op: &str) -> (&str, &str) {
    match segment_prefix_len(op) {
        Some(n) => op.split_at(n),
        None => ("", op),
    }
}

/// Whether `pos` in `t` starts a memory operand, allowing the segment-override
/// spellings `%gs:(%T)` / `%fs:8(%T)` the window scan must recognise.
///
/// The bare and indexed matchers look for `(%T)` / `(%T,` and require the
/// preceding byte to be an operand boundary.  Without this, a segment override
/// puts `:` before the `(` and the consumer is invisible to the fold — the
/// exact reason `fold_after_call` in tests/regression/
/// check_seg_const_offset_fold.sh saw `leaq` where GCC emits the folded form.
/// A four-byte validated prefix ending at `pos` moves the boundary to the byte
/// before it, so `movq %gs:(%T), %rax` is recognised while `foo:(%T)`,
/// `*%gs:(%T)` and `0x64::(%T)` are not.
fn at_mem_operand_boundary(t: &str, pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let b = t.as_bytes();
    if matches!(b[pos - 1], b' ' | b',' | b'\t') {
        return true;
    }
    if pos >= 4 && t.get(pos - 4..pos).and_then(segment_prefix_len).is_some() {
        return pos == 4 || matches!(b[pos - 5], b' ' | b',' | b'\t');
    }
    false
}

impl SibAddr {
    /// Parse `[disp][(][%base][, %index[, scale]][)]`.  Returns `None` for
    /// anything whose composition cannot be reasoned about (RIP-relative
    /// addressing, malformed fields, three register slots), so callers fail
    /// closed.
    fn parse(text: &str) -> Option<Self> {
        let open = text.find('(')?;
        let close = text.rfind(')')?;
        if close <= open {
            return None;
        }
        let (sym, disp) = parse_disp(&text[..open])?;
        let inner = text[open + 1..close].trim();
        // x86-64 RIP-relative addressing is ModRM mod=00 r/m=101: there is no
        // SIB byte, so no index register can be encoded.  An assembler may
        // silently DROP the index instead of erroring (observed:
        // `leaq V(%rip,%r11), %r10` encoded as plain `leaq V(%rip), %r10` --
        // wide_cond_zero_test wrong-code).  Reject it outright.
        if inner.contains("%rip") {
            return None;
        }
        let mut fields = inner.split(',');
        let base = fields.next()?.trim();
        let base = if base.is_empty() {
            None
        } else {
            if !base.starts_with('%') {
                return None;
            }
            Some(base.to_string())
        };
        let mut index = None;
        if let Some(idx) = fields.next() {
            let idx = idx.trim();
            if !idx.starts_with('%') {
                return None;
            }
            let scale = match fields.next() {
                Some(s) => match s.trim() {
                    "1" => 1,
                    "2" => 2,
                    "4" => 4,
                    "8" => 8,
                    _ => return None,
                },
                // A register in the second slot with no explicit scale is
                // scale 1 (`(%rax,%rbx)`).
                None => 1,
            };
            // A fourth field would be a second index: no encoding.
            if fields.next().is_some() {
                return None;
            }
            index = Some((idx.to_string(), scale));
        } else if fields.next().is_some() {
            return None;
        }
        // A parenthesised operand with no register at all (`()`) is malformed,
        // and a register-less sum is not an address composition.  Reject so
        // compose_sib can never be handed (or produce) a degenerate operand.
        if base.is_none() && index.is_none() {
            return None;
        }
        Some(SibAddr {
            sym,
            disp,
            base,
            index,
        })
    }

    /// Displacement text, empty when it is zero (GAS's canonical form).
    fn disp_text(&self) -> String {
        match &self.sym {
            None => {
                if self.disp == 0 {
                    String::new()
                } else {
                    self.disp.to_string()
                }
            }
            Some(sym) => {
                if self.disp == 0 {
                    sym.clone()
                } else if self.disp > 0 {
                    format!("{}+{}", sym, self.disp)
                } else {
                    format!("{}{}", sym, self.disp)
                }
            }
        }
    }

    /// Render back to assembly text in a form GAS and the integrated
    /// assembler both accept.
    fn render(&self) -> String {
        let d = self.disp_text();
        match (&self.base, &self.index) {
            (None, None) => {
                if d.is_empty() {
                    "0".to_string()
                } else {
                    d
                }
            }
            (Some(b), None) => format!("{}({})", d, b),
            // Scale 1 is implicit in the AT&T form and is omitted, matching
            // the emitter's canonical output (`8(%rsp, %r8)`, not
            // `8(%rsp, %r8, 1)`).  Emitting it would also bloat every folded
            // operand and change output that other passes pattern-match on.
            (None, Some((i, 1))) => format!("{}(,{})", d, i),
            (None, Some((i, s))) => format!("{}(,{}, {})", d, i, s),
            (Some(b), Some((i, 1))) => format!("{}({}, {})", d, b, i),
            (Some(b), Some((i, s))) => format!("{}({}, {}, {})", d, b, i, s),
        }
    }
}

/// Compose `outer` around `inner`: the consumer operand
/// `outer.disp(%T, outer.index, outer.scale)` carries `%T` in its BASE slot,
/// and `%T` holds the value of the producer address `inner`.  The sum is
///
/// ```text
///   outer.disp + inner.disp + inner.base
///               + inner.index*inner.scale + outer.index*outer.scale
/// ```
///
/// A SIB byte carries ONE base (always scale 1) and ONE index (scale
/// 1/2/4/8), so the composition is representable only when the summed terms
/// need at most two distinct registers and at most one of them is scaled.
/// Anything else returns `None` and the fold must not happen: emitting it
/// would give an instruction with no encoding, and the assembler may silently
/// TRUNCATE it into a wrong-but-assemblable one (observed:
/// `leaq 0(,%r11,4, %r10, 1), %r9` assembled as `leaq 0(,%r11,4), %r9`,
/// dropping `+ %r10` -- the div/mod-by-constant miscompile at -O1).
fn compose_sib(inner: &SibAddr, outer: &SibAddr) -> Option<String> {
    let mut terms: Vec<(String, u8)> = Vec::with_capacity(4);
    if let Some(b) = &inner.base {
        terms.push((b.clone(), 1));
    }
    if let Some((i, s)) = &inner.index {
        terms.push((i.clone(), *s));
    }
    if let Some(b) = &outer.base {
        terms.push((b.clone(), 1));
    }
    if let Some((i, s)) = &outer.index {
        terms.push((i.clone(), *s));
    }
    // Merge repeated registers: `(%rax) + (%rax,%rbx,2)` folds the two `%rax`
    // terms into one only when the summed scale is still a legal index scale.
    let mut merged: Vec<(String, u8)> = Vec::with_capacity(4);
    for (reg, scale) in terms {
        match merged.iter_mut().find(|(r, _)| *r == reg) {
            Some(slot) => {
                let sum = slot.1 as u32 + scale as u32;
                if !matches!(sum, 1 | 2 | 4 | 8) {
                    return None;
                }
                slot.1 = sum as u8;
            }
            None => merged.push((reg, scale)),
        }
    }
    if merged.len() > 2 {
        return None;
    }
    // Exactly one term may carry a scale != 1, and it becomes the index.
    let scaled: Vec<usize> = merged
        .iter()
        .enumerate()
        .filter(|(_, (_, s))| *s != 1)
        .map(|(i, _)| i)
        .collect();
    // A term in the BASE slot must be unscaled: the encoding has no scale
    // field there, so only its register name survives.
    let (base, index) = match (merged.len(), scaled.len()) {
        (0, _) => (None, None),
        (1, 0) => {
            let (b, _) = merged.remove(0);
            (Some(b), None)
        }
        (1, 1) => (None, Some(merged.remove(0))),
        (2, 0) => {
            let (b, _) = merged.remove(0);
            let i = merged.remove(0);
            (Some(b), Some(i))
        }
        (2, 1) => {
            let idx = merged.remove(scaled[0]);
            let (b, bscale) = merged.remove(0);
            if bscale != 1 {
                return None;
            }
            (Some(b), Some(idx))
        }
        _ => return None,
    };
    // Two symbolic displacements cannot be summed; one of them survives
    // verbatim (`leaq table(%rcx), %r8` folded into `8(%r8,%rbp,8)` gives
    // `table+8(%rcx, %rbp, 8)`).
    let sym = match (&inner.sym, &outer.sym) {
        (Some(_), Some(_)) => return None,
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (None, None) => None,
    };
    let disp = inner.disp.checked_add(outer.disp)?;
    // A ModRM/SIB displacement is a sign-extended 32-bit field.  A numeric
    // sum outside it has no encoding; with a symbol the linker range-checks
    // the relocation instead.
    if sym.is_none() && i32::try_from(disp).is_err() {
        return None;
    }
    Some(
        SibAddr {
            sym,
            disp,
            base,
            index,
        }
        .render(),
    )
}

/// Byte offset in `line` of the first character of the operand containing
/// `pos`: just past the previous operand separator, or just past the mnemonic
/// when the operand is the first one.  Needed because the index-relay fold
/// can meet its consumer in ANY operand position, and a store
/// (`movq %rax, (%rcx,%rbp,8)`) has a register operand before it.
fn operand_start(line: &str, pos: usize) -> usize {
    if let Some(c) = line[..pos].rfind(',') {
        return c + 1;
    }
    line.find(|c: char| c.is_whitespace()).unwrap_or(0)
}

/// Fold a producer `leaq` whose value is `producer_text` into the indexed
/// consumer operand `line[pos..=close]`, which holds `dst_text` in its base
/// slot.  Returns the composed operand, or `None` when the sum needs more
/// register slots than the encoding has (in which case the fold is skipped
/// and both instructions are kept).
fn compose_indexed_relay(
    line: &str,
    pos: usize,
    close: usize,
    dst_text: &str,
    producer_text: &str,
) -> Option<String> {
    let producer = SibAddr::parse(producer_text)?;
    let op_start = operand_start(line, pos);
    let cons_text = format!("{}{}", &line[op_start..pos], &line[pos..=close]);
    // The consumer may carry a segment override (`movq %gs:(%T, %idx, 4), %rax`).
    // It sits between the operand boundary and the displacement, and the caller
    // keeps `line[..pos]` in the rewritten line — so it is stripped here and
    // never becomes part of the composed operand.
    let trimmed = cons_text.trim_start();
    let (cons_text, has_seg) = match segment_prefix_len(trimmed) {
        Some(n) => (trimmed[n..].to_string(), true),
        None => (cons_text, false),
    };
    let cons = SibAddr::parse(&cons_text)?;
    // The base slot must be exactly %T.  Comparing the parsed field rather
    // than a text prefix also closes the `%r1` vs `%r10` prefix hazard.
    if cons.base.as_deref() != Some(dst_text) {
        return None;
    }
    let dst_fam = register_family_fast(dst_text);
    if cons
        .index
        .as_ref()
        .is_some_and(|(idx, _)| register_family_fast(idx) == dst_fam)
    {
        return None;
    }
    // %T is substituted away, so only the consumer's own displacement and
    // index survive into the sum.
    let cons_rest = SibAddr {
        sym: cons.sym,
        disp: cons.disp,
        base: None,
        index: cons.index,
    };
    let composed = compose_sib(&producer, &cons_rest)?;
    // The caller keeps the segment override, so an indexed result under a
    // segment prefix is a segment SIB: refuse it when the kill switch is set.
    if has_seg && composed.contains(',') && !seg_sib_enabled() {
        return None;
    }
    Some(composed)
}

/// Substitute the address computed by `leaq PRODUCER, %T` into the single
/// memory operand of `line` that carries `%T` in its BASE slot, whatever that
/// operand's displacement and index are, and return the rewritten line.
///
/// The consumer's displacement is ADDED to the producer's through
/// `compose_sib`; it is never spliced as text.  The textual splice this
/// replaces turned `leaq 8(%r13, %r10), %r9; movq 8(%r9), %rcx` into
/// `movq 88(%r13, %r10), %rcx` -- the consumer's `8` glued in front of the
/// producer's `8(` (sqlite3GenerateColumnNames at -O1 loaded the column
/// name from the wrong struct field).
///
/// Fails closed (`None`) when:
///   * `%T` is not the base of exactly one memory operand -- two such operands
///     are not rewritten half-way;
///   * the consumer operand is not a plain `[disp](%T[, %idx[, s]])`: an
///     indirect-branch operand (`*8(%T)`) or any other prefix keeps its text in
///     the displacement slot, where `parse_disp` would otherwise treat it as a
///     symbol.  A validated segment override (`%gs:8(%T)`) is the one prefix
///     that is composed *through*: it is stripped for the parse and re-attached
///     in front of the composed address, which is what makes the per-CPU
///     `leaq 8(%rax), %rcx; movq %gs:(%rcx), %rax` fold into GCC's
///     `movq %gs:8(%rax), %rax`;
///   * the sum needs more registers than one SIB byte encodes, two symbols, or
///     a numeric displacement outside disp32.
///
/// The caller still owns the liveness proof and must reject the result when it
/// mentions `%T` anywhere else (a remaining read of the deleted LEA's value).
/// `CCC_NO_SEG_SIB` kill switch, read once per process.  Shared by the
/// peephole folds below and the codegen emitters (`supports_seg_indexed_fold`).
pub(super) fn seg_sib_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CCC_NO_SEG_SIB").is_none())
}

pub(super) fn splice_lea_into_mem_operand(
    line: &str,
    dst_text: &str,
    producer_text: &str,
) -> Option<String> {
    let bare = format!("({})", dst_text);
    let indexed = format!("({},", dst_text);
    let mut hits = line
        .match_indices(&bare)
        .chain(line.match_indices(&indexed))
        .map(|(pos, _)| pos);
    let pos = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    let close = pos + line[pos..].find(')')?;
    let op_start = operand_start(line, pos);
    let raw_disp = line[op_start..pos].trim();
    // `%gs:(%T)` / `%fs:8(%T)`: keep the validated override (it belongs to the
    // same memory operand and the fold does not move it) and compose only the
    // address text behind it.  Anything else in the displacement slot is still
    // refused — `*%gs:(%T)` keeps its `*` here and falls out below.
    let (seg, disp_text) = match segment_prefix_len(raw_disp) {
        Some(n) => (&raw_disp[..n], &raw_disp[n..]),
        None => ("", raw_disp),
    };
    if disp_text.contains(['%', ':', '*', '(', ')', '$', ' ', '\t']) {
        return None;
    }
    let producer = SibAddr::parse(producer_text)?;
    let cons = SibAddr::parse(&format!("{}{}", disp_text, &line[pos..=close]))?;
    if cons.base.as_deref() != Some(dst_text) {
        return None;
    }
    let dst_fam = register_family_fast(dst_text);
    if cons
        .index
        .as_ref()
        .is_some_and(|(idx, _)| register_family_fast(idx) == dst_fam)
    {
        return None;
    }
    let cons_rest = SibAddr {
        sym: cons.sym,
        disp: cons.disp,
        base: None,
        index: cons.index,
    };
    let composed = compose_sib(&producer, &cons_rest)?;
    // A segment-prefixed operand that gains an index register is a segment SIB
    // (`%gs:(%base, %idx, scale)`).  Honour the `CCC_NO_SEG_SIB` kill switch
    // here as well: the emitters are gated, but this peephole composes the
    // LEA into the operand on its own, so without this check the switch would
    // leave segment SIB operands behind.
    if !seg.is_empty() && composed.contains(',') && !seg_sib_enabled() {
        return None;
    }
    // Keep the whitespace that separated the operand from the mnemonic or the
    // previous comma; replace exactly `disp(...)`.
    let lead = line[op_start..pos].len() - line[op_start..pos].trim_start().len();
    let start = op_start + lead;
    Some(format!(
        "{}{}{}{}",
        &line[..start],
        seg,
        composed,
        &line[close + 1..]
    ))
}

/// Requirements: no barrier, no implicit register traffic and no write to
/// `%base` between the two lines; the only mention of `%T` in the window is the
/// bare `(%T)` operand being folded; the rewritten line no longer mentions
/// `%T`; and `%T` is provably dead afterwards.
pub(super) fn fold_lea_into_load(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if debug_transparent(store, infos[i], i) || infos[i].pinned {
            i += 1;
            continue;
        }
        let lea = infos[i].trimmed(store.get(i)).to_string();
        let Some((addr_text, dst_text, addr_fams)) = parse_lea_address(&lea) else {
            i += 1;
            continue;
        };
        let dst_fam = register_family_fast(dst_text);
        // SELF-BASE LEAs (`leaq 16(%rax), %rax` — the strength-reduced
        // marching pointer the RA emits for `&base[i]` webs) are foldable
        // with three extra guarantees the normal form gets for free:
        //
        // 1. NO LIVE RE-EXECUTION: deleting the LEA leaves the register
        //    holding the PRE-LEA value (the fold's consumer reads `16(%rax)`
        //    as the base), so a back edge that re-enters the LEA while dst
        //    still carries the previous iteration's value would freeze the
        //    recurrence. The blanket `copy_in_loop` veto is refined to the
        //    precise condition: the fold is safe iff EVERY back-edge target
        //    that can re-execute the LEA sees dst DEAD at block entry —
        //    dead-at-entry means every path to the LEA (which READS dst)
        //    redefines dst first, so the LEA's input never flows around the
        //    back edge. This is what makes the slot-reload form foldable
        //    (linux_rbtree B2: `movq 80(%rsp), %rax; leaq 16(%rax), %rax;
        //    movq $0, (%rax)` — the reload re-establishes the base every
        //    iteration) while the carried form stays rejected.
        // 2. WINDOW SOUNDNESS: an intermediate line that reads dst expects
        //    base+K and must break the window (the existing dst_mask
        //    branch), and one that writes dst makes the LEA irreproducible
        //    (the existing addr_mask branch — which now includes dst
        //    itself).
        // 3. DEADNESS: `provably_dead_lv` after the consumer still proves
        //    nothing reads the LEA's result (base+K) later; the base value
        //    itself needs no proof — it is live by construction (the LEA
        //    read it).
        //
        // The replacement necessarily MENTIONS dst (the folded address is
        // `16(%rax)`), so the normal "dst fully removed from the consumer"
        // check is replaced by "no dst mention OUTSIDE the spliced operand".
        let self_base = addr_fams.contains(&dst_fam);
        if !is_relayable_family(dst_fam)
            || (self_base && self_base_reexec_unsafe(store, infos, i, dst_fam, &lv))
        {
            i += 1;
            continue;
        }
        // Bare `(%T)` matches the plain-relay case; `(%T,` matches the
        // indexed-relay case: `leaq table(%rip), %rcx; movq (%rcx, %rbp, 8), %r13`
        // (hash_table, adler32 unrolled loops).
        let addr_pat = format!("({})", dst_text);
        let idx_pat = format!("({},", dst_text);
        let folded_addr = addr_text.to_string();
        let mut addr_mask = 0u16;
        for f in &addr_fams {
            addr_mask |= 1u16 << f;
        }
        let dst_mask = 1u16 << dst_fam;

        let mut j = i + 1;
        let mut window = 0;
        while j < len && window < LEA_WINDOW {
            if debug_transparent(store, infos[j], j) {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() || infos[j].pinned {
                break;
            }
            let t = infos[j].trimmed(store.get(j));
            if has_implicit_reg_usage(t) {
                break;
            }
            // A write to any register the address reads makes the LEA
            // irreproducible at the use site — on an INTERVENING line. The
            // consumer itself (the first line touching %T) may write one:
            // a ModRM effective address is computed from the registers'
            // values before the instruction writes its destination, so
            // `leaq (%rax,%rax,2), %r11; leaq 1(%r11), %rax` composes to
            // `leaq 1(%rax,%rax,2), %rax`. Excluded: %rsp writes and the
            // stack instructions (push/pop/call/ret adjust %rsp around the
            // access — `popq 8(%rsp)` addresses with the incremented %rsp),
            // and the self-base producer, whose re-execution proof assumes
            // the old rule.
            if infos[j].reg_refs & addr_mask != 0 {
                let w = get_dest_reg(&infos[j]);
                if w != REG_NONE && addr_fams.contains(&w) {
                    let is_consumer = infos[j].reg_refs & (1u16 << dst_fam) != 0;
                    let stack_op = ["push", "pop", "call", "ret", "leave", "enter"]
                        .iter()
                        .any(|m| t.starts_with(m));
                    if !is_consumer || self_base || w == REG_RSP || stack_op {
                        break;
                    }
                }
            }
            if infos[j].reg_refs & dst_mask != 0 {
                // A BARE `(%T)` operand or a base/offset operand `(%T, …)`
                // absorbs the LEA verbatim. A displaced `d2(%T)` must never be
                // SPLICED (that would produce `d2DISP(%base)`); it is COMPOSED
                // instead by `compose_displaced_relay` (d1 + d2 on a numeric
                // producer displacement), below.
                let mut matched: Option<(usize, usize, String)> = None;
                // Plain relay: `%gs:(%T)` takes the folded address verbatim.
                // An indexed address under a segment override is a segment
                // SIB, which the `CCC_NO_SEG_SIB` kill switch must refuse.
                let seg_sib_blocked = |pos: usize| {
                    !seg_sib_enabled()
                        && folded_addr.contains(',')
                        && segment_prefix_len(t[operand_start(t, pos)..pos].trim()).is_some()
                };
                if let Some((pos, _)) = t
                    .match_indices(&addr_pat)
                    .find(|(pos, _)| at_mem_operand_boundary(t, *pos) && !seg_sib_blocked(*pos))
                {
                    matched = Some((pos, pos + addr_pat.len() - 1, folded_addr.clone()));
                } else if let Some((pos, _)) = t
                    .match_indices(&idx_pat)
                    .find(|(pos, _)| at_mem_operand_boundary(t, *pos))
                {
                    if let Some(cl) = t[pos..].find(')').map(|c| pos + c)
                        && let Some(new_operand) =
                            compose_indexed_relay(t, pos, cl, dst_text, &folded_addr)
                    {
                        // COMPOSE, DO NOT CONCATENATE.  See compose_sib: the
                        // old code spliced the producer's operand TEXT into
                        // the base slot, which is only sound when the producer
                        // occupies the base slot itself.  `leaq 0(,%r11,4), %r8`
                        // folded into `leaq (%r8, %r10, 1), %r9` used to become
                        // `leaq 0(,%r11,4, %r10, 1), %r9` -- two index
                        // registers, no encoding -- which the integrated
                        // assembler truncated to `leaq 0(,%r11,4), %r9`,
                        // silently dropping `+ %r10`.  The correct composition
                        // is `leaq (%r10, %r11, 4), %r9`: one instruction, one
                        // index register, exactly what GCC and Clang emit.
                        matched = Some((pos, cl, new_operand));
                    }
                }
                if matched.is_none() && !self_base {
                    matched = compose_displaced_relay(t, &addr_pat, &folded_addr);
                }
                if let Some((op, cl, new_operand)) = matched {
                    let replacement = format!("{}{}{}", &t[..op], new_operand, &t[cl + 1..]);
                    // Normal form: the consumer must not mention dst at all
                    // after the splice. Self-base form: the folded address
                    // IS `K(%dst)`, so only the text OUTSIDE the spliced
                    // operand must be dst-free (a mention there would read
                    // the LEA's result — base+K — which the fold no longer
                    // produces).
                    let mentions_ok = if self_base {
                        !line_refs_family(&t[..op], dst_fam)
                            && !line_refs_family(&t[cl + 1..], dst_fam)
                    } else {
                        !line_refs_family(&replacement, dst_fam)
                    };
                    if replacement != t
                        && mentions_ok
                        && provably_dead_lv(&lv, store, infos, j, dst_fam, &[i, j])
                    {
                        mark_nop(&mut infos[i]);
                        replace_line(store, &mut infos[j], j, format!("    {}", replacement));
                        lv.refresh_at(store, infos, j);
                        changed = true;
                    }
                }
                break;
            }
            window += 1;
            j += 1;
        }
        i += 1;
    }
    changed
}

/// Compose a LEA into a DISPLACED use of its result: with the producer
/// `leaq d1(B,I,s), %T`, the consumer operand `d2(%T)` addresses
/// `T + d2 = B + I*s + (d1 + d2)`, i.e. `d1+d2(B,I,s)` — one operand, the same
/// address (both are 64-bit address computations; a 32-bit consumer such as
/// `leal 1(%T), %eax` keeps the low 32 bits of the same sum). Returns the
/// `(operand start, closing paren, new operand)` splice for the first match.
///
/// Fail-closed scope: decimal displacements on both sides (no `sym(%rip)`
/// producer — its displacement is a relocation expression), a plain operand
/// boundary before `d2` (no segment-prefixed consumer, so the segment-SIB
/// kill switch is never involved), and `d1 + d2` within the signed 32-bit
/// displacement field. The caller keeps every other proof (no write to the
/// address registers in the window, `%T` absent from the rewritten line,
/// `%T` provably dead).
fn compose_displaced_relay(
    t: &str,
    addr_pat: &str,
    producer: &str,
) -> Option<(usize, usize, String)> {
    let open = producer.find('(')?;
    let (d1_text, sib) = producer.split_at(open);
    if sib.contains("%rip") {
        return None;
    }
    let d1: i64 = if d1_text.trim().is_empty() {
        0
    } else {
        d1_text.trim().parse().ok()?
    };
    let b = t.as_bytes();
    for (pos, _) in t.match_indices(addr_pat) {
        let mut start = pos;
        while start > 0 && (b[start - 1].is_ascii_digit() || b[start - 1] == b'-') {
            start -= 1;
        }
        if start == pos || !(start == 0 || matches!(b[start - 1], b' ' | b',' | b'\t')) {
            continue;
        }
        let Ok(d2) = t[start..pos].parse::<i64>() else {
            continue;
        };
        let sum = d1.checked_add(d2)?;
        if sum < i64::from(i32::MIN) || sum > i64::from(i32::MAX) {
            return None;
        }
        let disp = if sum == 0 {
            String::new()
        } else {
            sum.to_string()
        };
        return Some((start, pos + addr_pat.len() - 1, format!("{disp}{sib}")));
    }
    None
}

// ── Pass 2b: same-block redundant rip-LEA elimination ────────────────────────

/// Parse `leaq SYM(%rip), %R` → `(sym_text, dst_family)`. The address must be
/// the bare rip form with a symbolic displacement — no SIB, no base. Numeric
/// RIP displacements are deliberately excluded: `leaq 8(%rip), %r9` computes
/// the address relative to *that instruction's* next RIP, so identical text at
/// two instruction locations does not denote the same value.
fn parse_rip_symbol_lea(t: &str) -> Option<(String, RegId)> {
    let rest = t.strip_prefix("leaq ")?;
    let (addr, dst) = rest.rsplit_once(',')?;
    let (addr, dst) = (addr.trim(), dst.trim());
    let displacement = addr.strip_suffix("(%rip)")?;
    // `parse_disp` also accepts absolute numeric displacements. Only a true
    // relocatable symbol expression names a stable address across instructions.
    if !is_stable_symbol_expr(displacement) {
        return None;
    }
    let fam = plain_gp_operand(dst)?;
    if fam == REG_RSP || fam == REG_RBP || dst != REG_NAMES[0][fam as usize] {
        return None;
    }
    Some((addr.to_string(), fam))
}

/// `leaq sym(%rip), %R` is a pure constant materialization: when an earlier
/// instruction of the same straight-line run already materialized the same
/// symbol into `%H` and `%H` has not been rewritten since, the second LEA
/// computes a value the register file already holds.
///
/// * `%R == %H` → the redef is architecturally a no-op → delete it. This is
///   the remat duplicate the unrolled histogram loop emits four times per
///   iteration (`leaq bins(%rip), %r9` for every element) — each copy burns
///   an issue slot and a rename for a value already in flight.
/// * `%R != %H` → replace with `movq %H, %R` so the move-relay pass can fold
///   the copy into the uses (or dead-move elimination deletes it when the
///   register goes unread). A rename-conflicting consumer (the SIB index of
///   `addq $1, (%R, %H, 8)` already naming `%H`) simply keeps the copy —
///   correct, if less folded.
///
/// The home dies at ANY write of `%H` (a partial write corrupts the address
/// like a full one); barriers end the run (a jump target may enter with a
/// different home). Homes are tracked per symbol text: `leaq a(%rip)` and
/// `leaq b(%rip)` are independent constants.
pub(super) fn cse_symbol_lea(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();
    let mut homes: Vec<(String, RegId)> = Vec::new();
    for i in 0..len {
        if infos[i].is_nop() || infos[i].kind == LineKind::Empty {
            continue;
        }
        if infos[i].pinned || infos[i].is_barrier() {
            homes.clear();
            continue;
        }
        let t = infos[i].trimmed(store.get(i)).to_string();
        if let Some((sym, dst_fam)) = parse_rip_symbol_lea(&t) {
            // The LEA writes %R: any other symbol's home in %R dies first.
            homes.retain(|(s, f)| *f != dst_fam || *s == sym);
            if let Some(pos) = homes.iter().position(|(s, _)| *s == sym) {
                let home_fam = homes[pos].1;
                if home_fam == dst_fam {
                    // Value already in the destination register: no-op.
                    mark_nop(&mut infos[i]);
                    changed = true;
                } else {
                    let new_line = format!(
                        "    movq {}, {}",
                        REG_NAMES[0][home_fam as usize], REG_NAMES[0][dst_fam as usize]
                    );
                    replace_line(store, &mut infos[i], i, new_line);
                    changed = true;
                    homes[pos].1 = dst_fam;
                }
                continue;
            }
            homes.push((sym, dst_fam));
            continue;
        }
        // Any write to a home register retires that home.
        let writes = super::load_op_fuse::may_write_families(&t, &infos[i]);
        let mut k = 0;
        while k < homes.len() {
            if writes & (1u16 << homes[k].1) != 0 {
                homes.swap_remove(k);
            } else {
                k += 1;
            }
        }
    }
    changed
}

// ── Pass 3: producer retargeting (copy coalescing) ───────────────────────────

/// Pure producers: instructions whose ONLY effect is writing their trailing
/// register operand. Read-modify-write forms (`addl %ecx, %eax`) are excluded —
/// retargeting them would change which register is read.
const PURE_PRODUCERS: &[&str] = &[
    "movzbl ", "movzbq ", "movzwl ", "movzwq ", "movsbl ", "movsbq ", "movswl ", "movswq ",
    "movslq ", "movl ", "movq ", "leal ", "leaq ",
];

/// Width of the register a producer writes, from its destination operand text.
/// Only the 32- and 64-bit forms take part in retargeting.
fn dest_width(name: &str) -> Option<u8> {
    let fam = register_family_fast(name);
    if fam == REG_NONE || fam > REG_GP_MAX {
        return None;
    }
    if name == REG_NAMES[0][fam as usize] {
        Some(64)
    } else if name == REG_NAMES[1][fam as usize] {
        Some(32)
    } else {
        None
    }
}

/// Fold a producer + copy pair by making the producer write the copy's
/// destination directly:
///
/// ```text
///     movzbl (%rdx,%r12), %eax        movzbl (%rdx,%r12), %r10d
///     movq %rax, %r10            ->
/// ```
///
/// Conditions:
/// * the producer is a pure producer (no read-modify-write) and does not
///   mention the copy's destination family or (after the rewrite) its own
///   register -- the `aliasing` form below lifts this;
/// * the copy is an adjacent plain register move whose width is compatible —
///   a 32-bit producer may feed a `movl` or a `movq` copy (both leave the
///   destination zero-extended), a 64-bit producer only a `movq` copy
///   (retargeting a 64-bit producer under a `movl` copy would keep bits 32..63
///   that the copy discarded);
/// * the producer's register is provably dead after the copy (module header).
pub(super) fn retarget_producer_into_copy(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    retarget_producer_into_copy_impl(store, infos, false)
}

/// The late form of [`retarget_producer_into_copy`]: the producer's sources
/// may name either register. Every `PURE_PRODUCERS` form reads all of its
/// operands before writing its one destination and never reads that
/// destination implicitly (no `cmov`, no `bsf`), so
///
/// ```text
///     movzbl %al, %edx            movzbl %al, %eax
///     movq %rdx, %rax       ->
/// ```
///
/// and `movzbl %dl, %edx; movq %rdx, %rax` -> `movzbl %dl, %eax` are exact:
/// the read still happens at the producer, before any write. A resulting
/// `movq %R, %R` is deleted; `movl %R, %R` stays a zero-extension (the
/// self-zext cleanup removes it when the upper half is already zero).
///
/// Phase ordering: it runs once after the local fixpoints. Inside them it
/// would merge chains into one register before the RMW coalescing and the
/// copy+shift -> `lea` folds see them (`sarl; movslq %eax, %r11; movq %r11,
/// %rax; shlq $2, %rax; movq %rax, %rcx` loses its `leaq 0(,%r11,4), %rcx`),
/// so here it only removes copies those passes left behind.
pub(super) fn retarget_producer_into_copy_aliasing(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    retarget_producer_into_copy_impl(store, infos, true)
}

fn retarget_producer_into_copy_impl(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    aliasing: bool,
) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut rbp_roles = RbpRoles::default();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if debug_transparent(store, infos[i], i) || infos[i].pinned {
            i += 1;
            continue;
        }
        let prod = infos[i].trimmed(store.get(i)).to_string();
        let Some(op) = PURE_PRODUCERS.iter().find(|p| prod.starts_with(**p)) else {
            i += 1;
            continue;
        };
        let Some((prod_src, prod_dst)) = split_two_operands(&prod[op.len()..]) else {
            i += 1;
            continue;
        };
        let (Some(a_fam), Some(prod_w)) = (plain_gp_operand(prod_dst), dest_width(prod_dst)) else {
            i += 1;
            continue;
        };
        // `%rbp` participates only when it is a plain GPR in this function
        // (see `rbp_is_gpr_in_function`); the frame-pointer build keeps the
        // conservative `is_relayable_family` refusal. The function-range scan
        // runs lazily: candidates without an rbp operand never pay for it.
        let rbp_gpr = if a_fam == REG_RBP {
            rbp_roles.is_gpr(&lv, store, infos, i)
        } else {
            false
        };
        if !is_writable_family_gpr(a_fam, rbp_gpr) {
            i += 1;
            continue;
        }
        // Next real instruction(s): up to 2 same-width RMWs on `a_fam` when the
        // producer is a memory load, then the copy.
        let allow_rmws = prod_src.contains('(') && !op.starts_with("lea");
        let mut rmws: Vec<(usize, &'static str, String, u8)> = Vec::new();
        let mut j = i + 1;
        loop {
            while j < len && debug_transparent(store, infos[j], j) {
                j += 1;
            }
            if !allow_rmws
                || j >= len
                || infos[j].pinned
                || infos[j].is_barrier()
                || rmws.len() >= 2
            {
                break;
            }
            let tj = infos[j].trimmed(store.get(j));
            if has_implicit_reg_usage(tj) {
                break;
            }
            if let Some(rmw_op) = RMW_CONSUMER_OPS.iter().find(|p| tj.starts_with(**p)) {
                if let Some((r_src, r_dst)) = split_two_operands(&tj[rmw_op.len()..]) {
                    if plain_gp_operand(r_dst) == Some(a_fam)
                        && !line_refs_family(r_src, a_fam)
                        && (!r_src.starts_with('%') || r_src != "%cl")
                    {
                        if let Some(rw) = dest_width(r_dst) {
                            rmws.push((j, *rmw_op, r_src.to_string(), rw));
                            j += 1;
                            continue;
                        }
                    }
                }
            }
            break;
        }
        while j < len && debug_transparent(store, infos[j], j) {
            j += 1;
        }
        if j >= len || infos[j].pinned || infos[j].is_barrier() {
            i += 1;
            continue;
        }
        let copy = infos[j].trimmed(store.get(j)).to_string();
        let (copy_w, crest) = if let Some(r) = copy.strip_prefix("movq ") {
            (64u8, r)
        } else if let Some(r) = copy.strip_prefix("movl ") {
            (32u8, r)
        } else {
            i += 1;
            continue;
        };
        let Some((copy_src, copy_dst)) = split_two_operands(crest) else {
            i += 1;
            continue;
        };
        if register_family_fast(copy_src) != a_fam || plain_gp_operand(copy_src).is_none() {
            i += 1;
            continue;
        }
        let Some(d_fam) = plain_gp_operand(copy_dst) else {
            i += 1;
            continue;
        };
        if d_fam == a_fam {
            i += 1;
            continue;
        }
        let rbp_gpr = rbp_gpr || (d_fam == REG_RBP && rbp_roles.is_gpr(&lv, store, infos, i));
        if !is_writable_family_gpr(d_fam, rbp_gpr) {
            i += 1;
            continue;
        }
        // A 64-bit producer under a 32-bit copy would keep the upper half.
        if prod_w == 64 && copy_w == 32 {
            i += 1;
            continue;
        }
        if !rmws.iter().all(|(_, _, r_src, rw)| {
            *rw == prod_w && *rw == copy_w && !line_refs_family(r_src, d_fam)
        }) {
            i += 1;
            continue;
        }
        // The strict form keeps the producer's sources off D (the aliasing
        // form allows it; see `retarget_producer_into_copy_aliasing`).
        if (!aliasing && line_refs_family(prod_src, d_fam))
            || (!rmws.is_empty() && line_refs_family(prod_src, d_fam))
            || line_refs_family(copy_dst, a_fam)
        {
            i += 1;
            continue;
        }
        let mut owned = vec![i, j];
        for &(ri, _, _, _) in &rmws {
            owned.push(ri);
        }
        if !provably_dead_lv(&lv, store, infos, j, a_fam, &owned) {
            i += 1;
            continue;
        }
        // Retarget: the producer keeps its own width, written into D.
        let new_dst = if prod_w == 64 {
            REG_NAMES[0][d_fam as usize]
        } else {
            REG_NAMES[1][d_fam as usize]
        };
        let new_line = format!("    {}{}, {}", op, prod_src, new_dst);
        if !aliasing && line_refs_family(&new_line, a_fam) {
            i += 1;
            continue;
        }
        if rmws.is_empty() && op.trim_end() == "movq" && prod_src == new_dst {
            // `movq %rax, %rdx; movq %rdx, %rax`: the pair is the identity.
            mark_nop(&mut infos[i]);
            mark_nop(&mut infos[j]);
            lv.refresh_span(store, infos, i, j);
            rbp_roles.edited(&lv, store, infos, i);
            changed = true;
            i = j + 1;
            continue;
        }
        replace_line(store, &mut infos[i], i, new_line);
        for &(ri, rmw_op, ref r_src, rw) in &rmws {
            let r_dst = if rw == 64 {
                REG_NAMES[0][d_fam as usize]
            } else {
                REG_NAMES[1][d_fam as usize]
            };
            replace_line(
                store,
                &mut infos[ri],
                ri,
                format!("    {}{}, {}", rmw_op, r_src, r_dst),
            );
        }
        mark_nop(&mut infos[j]);
        lv.refresh_span(store, infos, i, j);
        rbp_roles.edited(&lv, store, infos, i);
        changed = true;
        i = j + 1;
    }
    changed
}

/// Upper bound on the lines between a chain's producer and its final copy
/// (keeps [`rename_chain_into_copy`] linear).
const CHAIN_WINDOW: usize = 8;

/// Does `line` name family `fam` at 16 or 8 bits (high byte included)? Those
/// spellings are partial writes / partial reads that a family rename would
/// either break (`shll %cl, …` -> `%al`) or change the meaning of.
fn names_family_narrow(line: &str, fam: RegId) -> bool {
    let f = fam as usize;
    if f >= REG_NAMES[0].len() {
        return true;
    }
    line.contains(REG_NAMES[2][f])
        || line.contains(REG_NAMES[3][f])
        || (f < 4 && line.contains(super::helpers::HIGH_BYTE_NAMES[f]))
}

/// Rename a short straight-line value chain into the register its final copy
/// targets, deleting the copy (the late, windowed generalisation of
/// [`retarget_producer_into_copy`], whose producer must be adjacent):
///
/// ```text
///     leal (%rax, %rax, 2), %r11d      leal (%rax, %rax, 2), %eax
///     addl %ebx, %r11d            ->   addl %ebx, %eax
///     movl %r11d, %eax
/// ```
///
/// Soundness, for the window `[i, j]` from the producer `i` to the copy `j`:
/// 1. The producer is a pure full-width write of `T` that does not read `T`,
///    so no line in the window observes `T`'s pre-window value.
/// 2. No line after the producer names `D` (any width), no window line has
///    implicit register effects (`implicit_reg_refs`,
///    `has_implicit_reg_usage`), and none is a barrier, label or pinned line:
///    `D`'s old value is unobserved after the producer and overwritten by the
///    copy anyway, so writing it from the producer on is unobservable. The
///    producer's SOURCES may name `D`: every `PURE_PRODUCERS` form reads all
///    operands before writing (the aliasing argument of
///    [`retarget_producer_into_copy_aliasing`]).
/// 3. Every `T` mention is a 32/64-bit name, so renaming every mention
///    `T -> D` (`replace_reg_family`) keeps each value bit-identical: `D`
///    holds exactly what `T` would have held at every point.
/// 4. Width: the copy zero-extends (`movl`) or copies (`movq`); if the last
///    write of `T` was 64-bit, a `movl` copy discarded bits 32..63 that the
///    renamed chain would keep, so that case is refused. A 32-bit last write
///    already left the upper half zero.
/// 5. `T` is provably dead after the copy (`provably_dead_lv`; the window
///    lines are the owned set).
///
/// Flags are unaffected (a register rename changes no computed value).
/// Phase: late, beside [`retarget_producer_into_copy_aliasing`] — inside the
/// local fixpoints it would merge chains before the RMW coalescers and LEA
/// folds see them.
pub(super) fn rename_chain_into_copy(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if debug_transparent(store, infos[i], i) || infos[i].pinned {
            i += 1;
            continue;
        }
        let prod = infos[i].trimmed(store.get(i));
        let Some(op) = PURE_PRODUCERS.iter().find(|p| prod.starts_with(**p)) else {
            i += 1;
            continue;
        };
        let Some((prod_src, prod_dst)) = split_two_operands(&prod[op.len()..]) else {
            i += 1;
            continue;
        };
        let (Some(t), Some(prod_w)) = (plain_gp_operand(prod_dst), dest_width(prod_dst)) else {
            i += 1;
            continue;
        };
        if matches!(t, 4 | 5) || t > REG_GP_MAX || line_refs_family(prod_src, t) {
            i += 1;
            continue;
        }
        // Walk forward to the copy `mov{l,q} %T, %D`.
        let mut window: Vec<usize> = vec![i];
        // The budget counts instructions only: debug directives (`.loc`)
        // ride in `window` but must not shorten the reach, or `-g` would
        // change the generated code.
        let mut real = 1usize;
        let mut last_w = prod_w;
        let mut copy_at = None;
        let mut k = i + 1;
        while k < len && real <= CHAIN_WINDOW {
            if infos[k].is_nop() {
                k += 1;
                continue;
            }
            let t_k = infos[k].trimmed(store.get(k));
            if debug_transparent(store, infos[k], k) {
                if line_refs_family(t_k, t) {
                    break;
                }
                window.push(k);
                k += 1;
                continue;
            }
            if infos[k].pinned || infos[k].is_barrier() || infos[k].kind == LineKind::InlineAsm {
                break;
            }
            // Implicit operands are invisible to the rename and to the D
            // check (`cltq` reads %eax): every window line must have none.
            if has_implicit_reg_usage(t_k) || implicit_reg_refs(t_k.as_bytes()) != 0 {
                break;
            }
            let copy_w = if t_k.starts_with("movq ") {
                Some(64u8)
            } else if t_k.starts_with("movl ") {
                Some(32u8)
            } else {
                None
            };
            if let Some(cw) = copy_w {
                if let Some((cs, cd)) = split_two_operands(&t_k[5..]) {
                    if plain_gp_operand(cs) == Some(t)
                        && plain_gp_operand(cd).is_some_and(|d| d != t)
                    {
                        copy_at = Some((k, cw, plain_gp_operand(cd).unwrap_or(t)));
                        break;
                    }
                }
            }
            if !line_refs_family(t_k, t) {
                // Unrelated lines may sit in the window as long as they leave
                // D alone (checked once D is known).
                window.push(k);
                real += 1;
                k += 1;
                continue;
            }
            if names_family_narrow(t_k, t) {
                break;
            }
            if writes_family(&infos[k], t_k, t) {
                let last = t_k.rsplit_once(',').map_or_else(
                    || t_k.split_whitespace().last().unwrap_or(""),
                    |(_, d)| d.trim(),
                );
                match dest_width(last) {
                    Some(w) if register_family_fast(last) == t => last_w = w,
                    _ => break,
                }
            }
            window.push(k);
            real += 1;
            k += 1;
        }
        let Some((j, copy_w, d)) = copy_at else {
            i += 1;
            continue;
        };
        if real < 2
            || matches!(d, 4 | 5)
            || !is_writable_family_gpr(d, false)
            || (last_w == 64 && copy_w == 32)
            || window[1..]
                .iter()
                .any(|&w| line_refs_family(infos[w].trimmed(store.get(w)), d))
        {
            i += 1;
            continue;
        }
        let mut owned = window.clone();
        owned.push(j);
        if !provably_dead_lv(&lv, store, infos, j, t, &owned) {
            i += 1;
            continue;
        }
        for &w in &window {
            let text = infos[w].trimmed(store.get(w));
            if line_refs_family(text, t) {
                let renamed = replace_reg_family(text, t, d);
                // A producer that copies `D` itself (`movq %r13, %rdi; rolq
                // $7, %rdi; movq %rdi, %r13`) renames to `movq %r13, %r13`:
                // a 64-bit self-move writes nothing and touches no flags, so
                // it goes.  `movl %r13d, %r13d` stays — it zero-extends,
                // exactly as the original `movl %r13d, %edi` did.
                if w == i && is_q_self_move(&renamed) {
                    mark_nop(&mut infos[w]);
                } else {
                    replace_line(store, &mut infos[w], w, format!("    {renamed}"));
                }
            }
        }
        mark_nop(&mut infos[j]);
        lv.refresh_span(store, infos, i, j);
        changed = true;
        i = j + 1;
    }
    changed
}

/// True iff `insn` is `movq %R, %R` for one 64-bit general register.
fn is_q_self_move(insn: &str) -> bool {
    insn.strip_prefix("movq ")
        .and_then(split_two_operands)
        .is_some_and(|(a, b)| a == b && a.starts_with('%') && dest_width(a) == Some(64))
}

/// Fold the loop-latch increment / pointer-bump shape that neither relay
/// pass can touch:
///
/// ```text
///     leaq 1(%rbx), %r11        leaq 1(%rbx), %rbx
///     movq %r11, %rbx      ->
///     cmpq %rdx, %r11           cmpq %rdx, %rbx
///     jb  .LBB2                 jb  .LBB2
/// ```
///
/// Why the existing passes decline it:
/// * `eliminate_move_relays` rewrites *uses of the copy destination* to the
///   copy source — here `%rbx` is live across the back edge, so it is never
///   dead and the rewrite cannot fire;
/// * `retarget_producer_into_copy` requires the producer's register to be
///   dead after the copy — here `%r11` feeds the loop-exit `cmp`.
///
/// The transform retargets the LEA's destination to the copy's destination
/// (the LEA's own base) and renames reads of the producer register inside
/// the following barrier-delimited region. Soundness:
///
/// 1. x86 `lea` reads all source operands before writing its destination, so
///    aliasing the destination with the base (`leaq 1(%rbx), %rbx`) is a
///    well-defined increment; neither `lea` nor `mov` touches flags.
/// 2. From the copy's position up to the first write of either family,
///    `%rA == %rB ==` the producer value at every point, so renaming reads
///    of `%rB` to `%rA` inside that region is value-preserving — including
///    lines whose destination is `%rA` (both operands hold the same value).
///    Read-modify-writes of `%rB` would land their result in the wrong
///    register and abort the candidate, as do memory-operand mentions of
///    `%rB` (only plain-register mentions are renamed).
/// 3. After the last rewrite, `%rB` must be provably dead
///    (`provably_dead_lv`: the `FileLiveness` dataflow answer or one of the
///    two syntactic proofs) — no path may read `%rB` again without an
///    intervening write, back edges, `call` argument registers and `ret`
///    included. If the proof fails, every rewrite is rolled back textually
///    and the original pair stays.
///
/// A bounded same-block gap between the LEA and copy is allowed only when
/// every intervening instruction is independent of both the LEA result and
/// the destination/base family. This lets loops retarget `lea step(base), tmp`
/// even when unrelated accumulator work sits between it and `mov tmp, base`,
/// without changing the order of any observable read/write.
///
/// Only copies whose destination family is referenced by the LEA's source
/// are handled here; the unaliased producer+copy case belongs to
/// `retarget_producer_into_copy`.
pub(super) fn fold_copy_into_lea_base(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    // NOTE ON A REMOVED DEAD STORE.  This function used to carry
    // `let mut lv = FileLiveness::new(store, infos);` plus `lv = lv2;` on the
    // success path, under an `#[allow(unused_variables, unused_assignments)]`
    // and a comment asserting the assignment was load-bearing because deleting
    // it "measurably regressed codegen (sqlite_varint 256 -> 270 insns,
    // gzip_crc32 65 -> 69) because it changed this loop's control flow".
    //
    // That claim was false and the reasoning was unsound.  `lv` was written
    // twice and never read, `FileLiveness` has no `Drop` impl, so the store was
    // inert by construction -- and a dead store in the *compiler's* source
    // cannot change the compiler's *behaviour* in any case; LCCC is a
    // deterministic function of its input, so "control flow in this loop" is
    // not a mechanism that exists.  Measured, deleting it is a no-op: all 150
    // asm outputs over tests/benchmark/programs + kernel_corpus at -O2 and
    // -O2 -march=x86-64-v3 are byte-identical before and after, and the two
    // cited files count 410 and 86 instructions either way (so the quoted 256
    // and 65 do not correspond to a whole-TU count on this tree at all).  It
    // also builds with zero warnings once the store is gone, so the `allow`
    // was covering nothing but the store itself.
    //
    // Recorded rather than deleted silently because the measurement was
    // presumably real for whoever took it -- most likely confounded by an
    // unrelated edit or a different binary -- and an unexplained deletion
    // invites someone to restore it.  The general rule it violates: a codegen
    // delta attributed to a provably dead store is a measurement error, not a
    // compiler quirk, and the fix is to re-measure, not to pin the dead code.
    let mut changed = false;
    let mut i = 0;
    while i < len {
        let li = i;
        i += 1;
        if infos[li].is_nop() || infos[li].pinned || infos[li].is_barrier() {
            continue;
        }
        let lea = infos[li].trimmed(store.get(li)).to_string();
        let (lea_rest, prod_w) = if let Some(r) = lea.strip_prefix("leaq ") {
            (r, 64u8)
        } else if let Some(r) = lea.strip_prefix("leal ") {
            (r, 32u8)
        } else {
            continue;
        };
        if has_implicit_reg_usage(&lea) {
            continue;
        }
        let Some((lea_src, lea_dst)) = split_two_operands(lea_rest) else {
            continue;
        };
        if !lea_src.contains('(') {
            continue;
        }
        let Some(b_fam) = plain_gp_operand(lea_dst) else {
            continue;
        };
        if !is_relayable_family(b_fam) {
            continue;
        }
        // The copy of the LEA result may be separated by independent
        // instructions. Retargeting the LEA changes the base family before
        // the copy and stops defining the temporary, so every skipped line
        // must be free of both families (including implicit register uses),
        // and the bounded window must stay within one basic block.
        const MAX_RELAY_GAP: usize = 8;
        let b_mask = 1u16 << b_fam;
        let mut j = li + 1;
        let mut gap = 0usize;
        let mut independent_gap = Vec::new();
        loop {
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len || infos[j].pinned || infos[j].is_barrier() {
                break;
            }
            let tj = infos[j].trimmed(store.get(j));
            let is_copy_from_lea = ["movq ", "movl "]
                .iter()
                .filter_map(|prefix| tj.strip_prefix(prefix))
                .filter_map(split_two_operands)
                .any(|(src, _)| register_family_fast(src) == b_fam);
            if is_copy_from_lea {
                break;
            }
            if gap < MAX_RELAY_GAP && infos[j].reg_refs & b_mask == 0 && !has_implicit_reg_usage(tj)
            {
                independent_gap.push(j);
                gap += 1;
                j += 1;
                continue;
            }
            break;
        }
        if j >= len || infos[j].pinned || infos[j].is_barrier() {
            continue;
        }
        let copy = infos[j].trimmed(store.get(j)).to_string();
        let (copy_w, copy_rest) = if let Some(r) = copy.strip_prefix("movq ") {
            (64u8, r)
        } else if let Some(r) = copy.strip_prefix("movl ") {
            (32u8, r)
        } else {
            continue;
        };
        let Some((copy_src, copy_dst)) = split_two_operands(copy_rest) else {
            continue;
        };
        // The copy must read exactly the LEA's destination family and write
        // a different relayable family that the LEA's source references
        // (the aliased case — unaliased retargeting is the other pass's job).
        if register_family_fast(copy_src) != b_fam || plain_gp_operand(copy_src).is_none() {
            continue;
        }
        let Some(a_fam) = plain_gp_operand(copy_dst) else {
            continue;
        };
        if a_fam == b_fam || !is_relayable_family(a_fam) {
            continue;
        }
        let a_mask = 1u16 << a_fam;
        if independent_gap
            .iter()
            .any(|&idx| infos[idx].reg_refs & a_mask != 0)
        {
            continue;
        }
        // A 64-bit LEA under a 32-bit copy would drop the copy's truncation:
        // later 64-bit reads of %rB could not be renamed to the 32-bit
        // (zero-extended) %rA.
        if prod_w == 64 && copy_w == 32 {
            continue;
        }
        if !line_refs_family(lea_src, a_fam) {
            continue;
        }
        let new_dst_name = REG_NAMES[usize::from(prod_w == 32)][a_fam as usize];
        let new_lea = format!(
            "    {} {}, {}",
            if prod_w == 64 { "leaq" } else { "leal" },
            lea_src,
            new_dst_name
        );

        // --- rename window: reads of %rB until the first write of either
        // --- family or a barrier. Collect rewrites; abort on any shape we
        // --- cannot rename (RMW of %rB, memory mention, implicit usage).
        let b_mask = 1u16 << b_fam;
        let mut rewrites: Vec<(usize, String, String)> = Vec::new(); // (idx, orig, new)
        let mut abort = false;
        let mut k = j + 1;
        while k < len {
            if infos[k].is_nop() {
                k += 1;
                continue;
            }
            if infos[k].pinned || has_implicit_reg_usage(infos[k].trimmed(store.get(k))) {
                abort = true;
                break;
            }
            if infos[k].is_barrier() {
                break; // downstream paths are the deadness proof's job
            }
            let t = infos[k].trimmed(store.get(k)).to_string();
            if infos[k].reg_refs & b_mask == 0 {
                // A write of %rA closes the window (its value diverges from
                // %rB's); a later %rB read then fails the deadness proof and
                // rolls the candidate back.
                if get_dest_reg(&infos[k]) == a_fam {
                    break;
                }
                k += 1;
                continue;
            }
            // %rB redefined without reading itself: later reads see the new
            // def, not our value — the window ends.
            if is_full_write(&infos[k], &t, b_fam) {
                break;
            }
            // A read-modify-write of %rB would leave its result in the wrong
            // register after a rename.
            if get_dest_reg(&infos[k]) == b_fam {
                abort = true;
                break;
            }
            let Some(new_t) = rename_plain_family_reads(&t, b_fam, a_fam) else {
                abort = true;
                break;
            };
            // Raw stored line: restoring the trimmed matcher here stripped
            // the line's indentation whenever the deadness proof refused the
            // candidate (cosmetic only — the assembler ignores indentation —
            // but it pollutes assembly diffs and indent-anchored counters).
            rewrites.push((k, store.get(k).to_string(), new_t));
            k += 1;
        }
        if abort {
            continue;
        }

        // --- apply, prove, or roll back -----------------------------------
        let orig_lea = store.get(li).to_string();
        let orig_copy = store.get(j).to_string();
        replace_line(store, &mut infos[li], li, new_lea);
        mark_nop(&mut infos[j]);
        for &(idx, _, ref new_t) in &rewrites {
            replace_line(store, &mut infos[idx], idx, new_t.clone());
        }
        let lv2 = FileLiveness::new(store, infos);
        // The deadness query is anchored at the LEA line, NOT at the copy:
        // the copy is NOP-marked at this point and `FileLiveness` only marks
        // real instructions as known, so a query at its index would answer
        // `None` and silently degrade the proof to its syntactic fallbacks.
        // Every line between `li` and `j` is a NOP, so "dead after the LEA"
        // is exactly "dead after the (deleted) copy" on the rewritten text.
        if provably_dead_lv(&lv2, store, infos, li, b_fam, &[li, j]) {
            changed = true;
            i = j + 1;
        } else {
            replace_line(store, &mut infos[li], li, orig_lea);
            replace_line(store, &mut infos[j], j, orig_copy);
            for (idx, orig, _) in rewrites {
                replace_line(store, &mut infos[idx], idx, orig);
            }
        }
    }
    changed
}

// ── Pass: copy-into-RMW-consumer coalescing ──────────────────────────────

/// Two-operand ALU mnemonics (AT&T, destination last) eligible as the
/// consuming instruction of a coalesced copy. Every entry is strictly
/// two-operand — no `imul` three-operand form, no `lea` SIB reads — and reads
/// its source operands before writing its destination, so retargeting the
/// destination from the copy's destination family to its source family
/// preserves the computed value and the flags. Deliberately minimal: `adc`,
/// `sbb`, `rol` and `ror` are sound by the same argument but unmeasured —
/// extend only with census data.
const RMW_CONSUMER_OPS: &[&str] = &[
    "andl ", "orl ", "xorl ", "addl ", "subl ", "shll ", "shrl ", "sarl ", "andq ", "orq ",
    "xorq ", "addq ", "subq ", "shlq ", "shrq ", "sarq ",
];

/// Coalesce `mov %S, %D` into the read-modify-write consumer that destinations
/// `%D`, retargeting the consumer and renaming the following plain-register
/// reads of `%D` to `%S`:
///
/// ```text
///     movl %r8d, %r10d         andl $2080895, %r8d
///     andl $2080895, %r10d  -> orl %r8d, %edi
///     orl %r10d, %edi
/// ```
///
/// Why the sibling passes decline it:
/// * `eliminate_move_relays` rewrites uses of the copy destination to the
///   copy source only when the consumer's destination is a DIFFERENT family
///   (a pure source read) — here `%r10d` IS the consumer's destination;
/// * `retarget_producer_into_copy` requires a pure (non-RMW) producer;
/// * `coalesce_register_copies` handles only entry-block `movq` shuffles.
///
/// Soundness:
/// 1. The consumer is adjacent (NOPs apart). For a plain copy `%S == %D`
///    still holds there, and the destination width is at most the copy's
///    width — a 64-bit consumer under a 32-bit copy would read the source's
///    unknown upper half, with two proven exceptions: `movl %S, %D` +
///    `shlq $32, %D` folds (identical values; the divergent CF is
///    compensated by `shift_flags_safe_after`), and ANY 64-bit consumer folds
///    when `upper32_zero_at` proves the source's upper half zero (exact
///    value equality — flags included). For an extension copy the
///    consumer must be
///    `andl $mask` with the mask clearing every bit above the extension
///    width: below it both spellings agree (the extension's identity bits),
///    above it the mask forces zero in both. Either way the consumer reads
///    identical values before and after the destination retarget, hence
///    computes the identical result and flags (x86 reads all sources before
///    writing the destination; the deleted `mov` writes no flags).
/// 2. Every renamed line reads `%D` after the consumer wrote it, and `%S`
///    holds exactly `%D`'s current value there (the retargeted consumer
///    established it; every prior rename preserved it — including
///    read-modify-writes of `%D`, which compute the identical result+flags
///    from the same values into `%S`). A full redefinition of `%D` ends the
///    window; ANY mention of `%S` aborts it (a pre-existing read would
///    observe the new value instead of the old one, a write would clobber
///    the renamed value); partial writes (`movb`, `setcc`), `cmov`/`xchg`
///    and memory mentions of `%D` abort, as do pinned lines and lines with
///    implicit register traffic; the window ends at barriers, leaving
///    downstream paths to the deadness proof.
/// 3. The source's OLD value must be dead at the copy (`provably_dead_lv` on
///    the pre-rewrite text): the consumer's write unconditionally clobbers
///    the source family.
/// 4. After the rewrite, `%D` must be provably dead (`provably_dead_lv` on a
///    fresh analysis, anchored at the consumer line): the deleted copy no
///    longer defines it. On failure every rewrite is rolled back textually
///    and the original pair stays.
/// Parse an `and $imm, ...` source operand into its u32 mask. Accepts GAS
/// decimal/hex immediates (`$127`, `$0x7f`, `$-1`); anything else (symbols,
/// malformed) refuses so the caller falls back to `continue`.
/// Parse a `$`-immediate operand (`$32`, `$0x20`, `$-1`) into its 32-bit
/// value. Shared by the P1 mask check and the P2.5 shift-count check.
fn parse_imm32(src: &str) -> Option<u32> {
    let digits = src.strip_prefix('$')?;
    if digits.is_empty() {
        return None;
    }
    if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else if let Some(neg) = digits.strip_prefix('-') {
        neg.parse::<i32>().ok().map(|v| (-v) as u32)
    } else {
        digits.parse::<u32>().ok()
    }
}

/// P2.5's flag-compensation sets: the cross-family `movl; shlq $32` fold
/// computes identical VALUES but divergent FLAGS, so the fold stands only
/// when both divergent flags are provably unobservable (see
/// `shift_flags_safe_after`). Exactly two flags diverge:
/// * CF — defined by the ISA and divergent: 0 in the zero-extended spelling
///   vs the source's bit 32 in the full-width one;
/// * OF — architecturally UNDEFINED after a multi-bit shift, but
///   silicon-deterministic and value-dependent (it tracks the pre-shift MSB),
///   hence divergent whenever the source's bit 63 is set. Measured natively
///   (x86-64, 2026-09-13, 9 source values): src `0x8000_0000_8000_0000`
///   gives OF 0 unfolded vs 1 folded, and a following `adox` — which consumes
///   OF as its carry-in — returned 0 vs 1, i.e. a wrong VALUE, not just a
///   wrong flag. ZF/SF/PF need no audit: they are functions of the result and
///   both spellings compute the identical result. AF is undefined in both and
///   every AF reader (the BCD group) is already a `CF_READERS` veto.
///
/// "Undefined" licences a compiler to CLOBBER a flag, not to OBSERVE it: the
/// text this pass rewrites is lccc's own output, and lccc emits OF readers —
/// `setl`/`setle`/`setg`/`setge` for every signed compare
/// (`backend/x86/codegen/comparison.rs`), and `adox` through its own
/// assembler (`assembler/encoder/mod.rs`; `passes/liveness.rs` already lists
/// adcx/adox). The CF-only audit did not merely fail to credit them:
/// `cf_skip_prefix` skips the whole `set`/`cmov`/`j` prefix family, so an
/// OF-reading `setl` was stepped over as flag-irrelevant.
///
/// Comparisons are case-insensitive: the consumer matcher is case-sensitive
/// (missed folds are safe), but a missed flag READER would be unsound.
///
/// Carry-flag readers: any of these vetoes the fold.
/// * `adc`/`sbb`/`adcx` read CF as carry-in; `rcl`/`rcr` rotate through it;
///   `salc` materialises it; `cmc` COMPLEMENTS it (a read!);
/// * CF-conditional `set`/`cmov`/`j` (lowercase `jcc` never reaches the scan
///   — the classifier barriers it — but odd-case spellings do);
/// * whole-flag reads (`lahf`, `pushf*`, `syscall`/`sysenter` saving RFLAGS);
/// * the BCD group (invalid in 64-bit mode; `daa`/`das` read CF, the rest
///   read AF which the fold leaves undefined) and deliberate traps (`int*`
///   hands RFLAGS to a debugger/handler — stronger than industry, free).
const CF_READERS: &[&str] = &[
    "adc", "adcb", "adcw", "adcl", "adcq", "adcx", "adcxl", "adcxq", "sbb", "sbbb", "sbbw", "sbbl",
    "sbbq", "rcl", "rclb", "rclw", "rcll", "rclq", "rcr", "rcrb", "rcrw", "rcrl", "rcrq", "salc",
    "cmc", "aaa", "aas", "aam", "aad", "daa", "das",
    // CF-only setcc/cmovcc/jcc predicates (B/C/NAE, NB/NC/AE).
    "setb", "setc", "setnae", "setnb", "setnc", "setae", "cmovb", "cmovc", "cmovnae", "cmovnb",
    "cmovnc", "cmovae", "jb", "jc", "jnae", "jnb", "jnc", "jae",
    // COMBINED CF+ZF predicates read CF too. These were missing: the
    // `set`/`cmov`/`j` prefix skip below would otherwise classify them as
    // ZF/SF/OF-only reads and skip over a live carry observation (the
    // conditional branch forms are additionally caught as CFG barriers,
    // but setcc/cmovcc are not barrier lines).
    "seta", "setnbe", "setbe", "setna", "cmova", "cmovnbe", "cmovbe", "cmovna", "ja", "jnbe", "jbe",
    "jna",
    // x87 FCMOV after FCOMI/FUCOMI (those deposit the comparison directly
    // in EFLAGS): the carry-conditional moves read EFLAGS CF. i387/i686
    // shapes primarily; the suffixless names are what GAS emits.
    "fcmova", "fcmovnbe", "fcmovae", "fcmovnb", "fcmovnc", "fcmovb", "fcmovc", "fcmovnae",
    "fcmovbe", "fcmovna", "lahf", "pushf", "pushfw", "pushfd", "pushfq", "syscall", "sysenter",
    "int", "int1", "int3", "into",
];

/// Unconditional carry-flag writers (any operands): a scan hit ends the
/// audit with success. Mul/div-family CF is architecturally undefined but
/// silicon-deterministic, hence identical in both spellings (their inputs
/// are — renames preserve values); `bt*` writes CF for every count.
const CF_CLOBBER: &[&str] = &[
    "add",
    "addb",
    "addw",
    "addl",
    "addq",
    "sub",
    "subb",
    "subw",
    "subl",
    "subq",
    "and",
    "andb",
    "andw",
    "andl",
    "andq",
    "or",
    "orb",
    "orw",
    "orl",
    "orq",
    "xor",
    "xorb",
    "xorw",
    "xorl",
    "xorq",
    "cmp",
    "cmpb",
    "cmpw",
    "cmpl",
    "cmpq",
    "test",
    "testb",
    "testw",
    "testl",
    "testq",
    "mul",
    "mulb",
    "mulw",
    "mull",
    "mulq",
    "imul",
    "imulb",
    "imulw",
    "imull",
    "imulq",
    "div",
    "divb",
    "divw",
    "divl",
    "divq",
    "idiv",
    "idivb",
    "idivw",
    "idivl",
    "idivq",
    "neg",
    "negb",
    "negw",
    "negl",
    "negq",
    "bt",
    "btw",
    "btl",
    "btq",
    "bts",
    "btsw",
    "btsl",
    "btsq",
    "btr",
    "btrw",
    "btrl",
    "btrq",
    "btc",
    "btcw",
    "btcl",
    "btcq",
    "clc",
    "stc",
    "sahf",
    "popf",
    "popfw",
    "popfd",
    "popfq",
    "xadd",
    "xaddb",
    "xaddw",
    "xaddl",
    "xaddq",
    "cmpxchg",
    "cmpxchgb",
    "cmpxchgw",
    "cmpxchgl",
    "cmpxchgq",
    "cmpxchg8b",
    "cmpxchg16b",
    "popcnt",
    "popcntw",
    "popcntl",
    "popcntq",
    "lzcnt",
    "lzcntw",
    "lzcntl",
    "lzcntq",
    "tzcnt",
    "tzcntw",
    "tzcntl",
    "tzcntq",
    "bsf",
    "bsfw",
    "bsfl",
    "bsfq",
    "bsr",
    "bsrw",
    "bsrl",
    "bsrq",
    "rdrand",
    "rdrandw",
    "rdrandl",
    "rdrandq",
    "rdseed",
    "rdseedw",
    "rdseedl",
    "rdseedq",
    "andn",
    "andnl",
    "andnq",
    "bextr",
    "bextrl",
    "bextrq",
    "bzhi",
    "bzhil",
    "bzhiq",
    "blsi",
    "blsil",
    "blsiq",
    "blsmsk",
    "blsmskl",
    "blsmskq",
    "blsr",
    "blsrl",
    "blsrq",
    "cmps",
    "cmpsb",
    "cmpsw",
    "cmpsl",
    "cmpsq",
    "scas",
    "scasb",
    "scasw",
    "scasl",
    "scasq",
    "comiss",
    "comisd",
    "ucomiss",
    "ucomisd",
    "vcomiss",
    "vcomisd",
    "vucomiss",
    "vucomisd",
    "fcomi",
    "fcomip",
    "fucomi",
    "fucomip",
    // SSE4.2 string/element compares deposit CF from the sign bit of the
    // intermediate result; PTEST/VPTEST write CF from the masked-compare.
    "pcmpistri",
    "pcmpestri",
    "pcmpistrm",
    "pcmpestrm",
    "ptest",
    "vptest",
    "iret",
    "iretd",
    "iretq",
    "sysret",
    "sysexit",
];

/// Overflow-flag readers: each vetoes the fold exactly like a CF reader.
/// Enumerated EXACTLY, with no suffix stripping — a missed reader is unsound,
/// and condition-code names collide with size suffixes: `setl` stripped of
/// its trailing `l` is `set`, which would silently drop the veto. The `j*`
/// forms are redundant (the classifier barriers conditional jumps) and are
/// listed so the set stays complete under a future classifier change.
const OF_READERS: &[&str] = &[
    // ADX: ADOX consumes OF as its carry-in and writes OF back (silicon:
    // post-OF independent of the preset; the f8 probe shows its RESULT
    // diverging with the shift's OF). ADCX is the CF twin and already vetoes
    // through `CF_READERS`. Suffixed spellings are listed because the tables
    // are exact-match: without them `adoxq` (what GAS emits) would fall
    // through to default-deny instead of being vetoed for the right reason.
    "adox", "adoxl", "adoxq",
    // setcc / cmovcc whose predicate reads OF: O, NO, and the signed
    // relations L/NL/LE/NLE/G/NG/GE/NGE (SF != OF, SF == OF).
    "seto", "setno", "setl", "setnl", "setle", "setnle", "setg", "setng", "setge", "setnge",
    "cmovo", "cmovno", "cmovl", "cmovnl", "cmovle", "cmovnle", "cmovg", "cmovng", "cmovge",
    "cmovnge", "jo", "jno", "jl", "jnl", "jle", "jnle", "jg", "jng", "jge", "jnge",
    // INTO traps on OF. (It also appears in `CF_READERS`, where it vetoes for
    // the stronger reason that it hands RFLAGS to a handler.)
    "into",
    // Whole-RFLAGS readers: they observe OF as well as CF, so once CF is dead
    // they must still veto on OF grounds instead of riding the `push` prefix
    // skip. `lahf` is deliberately absent — it loads only flags bits 7..0
    // (SF:ZF:0:AF:0:PF:1:CF), so OF (bit 11) is never read.
    "pushf", "pushfw", "pushfd", "pushfq", "syscall", "sysenter", "int", "int1", "int3",
    // The BCD group leaves every flag it does not define undefined; vetoing is
    // the conservative reading (and they are invalid in 64-bit mode anyway).
    "daa", "das", "aaa", "aam", "aad", "aas",
];

/// Instructions that provably WRITE OF, so an OF reader past one observes the
/// writer's OF rather than the divergent shift OF. Verified on silicon
/// (native x86-64, 2026-09-13): post-OF independent of a preset OF=1/OF=0 for
/// every base below, with controls proving the harness could tell the
/// difference (a `mov` showed 1/0, an `add` 0/0).
///
/// Membership is tested against the token itself and against the token with
/// ONE trailing size letter removed, so `addq`/`andnl`/`popcntl` credit their
/// base. The base list contains only names that can never be a condition-code
/// or flag-neutral mnemonic, because the two failure modes are asymmetric: a
/// false CREDIT accepts the fold with OF still live (unsound), while a false
/// negative merely costs folds. Stripping is therefore safe here and unsafe
/// for `OF_READERS`.
///
/// Deliberately absent, each measured or reasoned:
/// * `clc`/`stc`/`cmc`/`sahf` and the `bt`/`bts`/`btr`/`btc` family — CF-only
///   writers; silicon shows them preserving OF. These are exactly the shapes
///   that made a single-flag audit unsound: they killed the CF credit and
///   ended the scan with OF still live.
/// * shifts and rotates — OF after a multi-bit shift/rotate is the very
///   value-dependent quantity under audit, so a shift may not credit OF;
/// * `div`/`idiv` (OF undefined), `mulx`/`pdep`/`pext`/`rorx`/`shlx`/`shrx`/
///   `sarx`/`crc32`/`not`/`bswap`/`lea`/`xchg` (flag-neutral on silicon),
///   `adcx` (writes CF, preserves OF);
/// * all SSE/AVX arithmetic — writes no flags; the size-letter rule cannot
///   miscredit them (`paddb` strips to `padd`, `psubb` to `psub`, neither a
///   base).
const OF_CLOBBER_BASE: &[&str] = &[
    "add",
    "sub",
    "adc",
    "sbb",
    "cmp",
    "test",
    "and",
    "or",
    "xor",
    "neg",
    "inc",
    "dec",
    "imul",
    "mul",
    "xadd",
    "cmpxchg",
    "andn",
    "blsi",
    "blsr",
    "blsmsk",
    "bzhi",
    "bextr",
    "popcnt",
    "lzcnt",
    "tzcnt",
    "ptest",
    "vptest",
    "pcmpistri",
    "pcmpestri",
    "pcmpistrm",
    "pcmpestrm",
    "popf",
    "popfw",
    "popfd",
    "popfq",
    "iret",
    "iretd",
    "iretq",
];

/// Instructions with provably no RFLAGS effect: the scan skips them.
/// (`inc*` is covered by the `in` prefix rule in `cf_skip_prefix`;
/// `shld`/`shrd` never READ CF, so skipping (losing clobber credit) is
/// safe; `movs*`/`ins*` ride the `mov`/`in` prefix rules.)
const CF_SKIP_EXACT: &[&str] = &[
    "lea",
    "xchg",
    "xchgb",
    "xchgw",
    "xchgl",
    "xchgq",
    "bswap",
    "movbe",
    "not",
    "notb",
    "notw",
    "notl",
    "notq",
    "dec",
    "decb",
    "decw",
    "decl",
    "decq",
    "cbw",
    "cwde",
    "cdqe",
    "cwd",
    "cdq",
    "cqo",
    "cqto",
    "cltq",
    "mulx",
    // PDEP/PEXT leave CF UNCHANGED (verified on silicon, Raptor Cove:
    // STC/PDEP/SETC -> 1 and CLC/PDEP/SETC -> 0); the BMI2 manual table that
    // suggests "CF cleared" describes SF/ZF/PF only. Skipping is sound and
    // crediting them as clobbers would be unsound.
    "pdep",
    "pdepl",
    "pdepq",
    "pext",
    "pextl",
    "pextq",
    "sarx",
    "shlx",
    "shrx",
    "rorx",
    "adox",
    "crc32",
    "crc32b",
    "crc32w",
    "crc32l",
    "crc32q",
    "stos",
    "stosb",
    "stosw",
    "stosl",
    "stosq",
    "lods",
    "lodsb",
    "lodsw",
    "lodsl",
    "lodsq",
    "loop",
    "loope",
    "loopne",
    "loopz",
    "loopnz",
    "jcxz",
    "jecxz",
    "jrcxz",
    "shld",
    "shldw",
    "shldl",
    "shldq",
    "shrd",
    "shrdw",
    "shrdl",
    "shrdq",
    "hlt",
    "cli",
    "sti",
    "clac",
    "stac",
    "nop",
    "pause",
    "lgdt",
    "lidt",
    "lldt",
    "ltr",
    "lmsw",
    "clts",
    "rdtsc",
    "rdtscp",
    "rdpmc",
    "rdmsr",
    "wrmsr",
    "cpuid",
    "xgetbv",
    "xsetbv",
    "rdfsbase",
    "rdgsbase",
    "wrfsbase",
    "wrgsbase",
    "xlat",
    "xlatb",
    "fxsave",
    "fxsave64",
    "fxrstor",
    "fxrstor64",
    "ldmxcsr",
    "stmxcsr",
    "xsave",
    "xsave64",
    "xrstor",
    "xrstor64",
    "monitor",
    "mwait",
    "mwaitx",
    "encls",
    "enclu",
];

/// Shifts/rotates write CF only when the effective count is nonzero (see
/// `shift_proves_cf_clobber`); `rcl`/`rcr` are readers (in `CF_READERS`).
const SHIFTS_GUARDED: &[&str] = &[
    "shl", "shlb", "shlw", "shll", "shlq", "shr", "shrb", "shrw", "shrl", "shrq", "sar", "sarb",
    "sarw", "sarl", "sarq", "sal", "salb", "salw", "sall", "salq", "rol", "rolb", "rolw", "roll",
    "rolq", "ror", "rorb", "rorw", "rorl", "rorq",
];

/// Prefix rules for the CF audit (checked AFTER the reader/clobber sets, so
/// the vetoed/credited members of each family win): every `mov`-form is
/// flag-free (moves, extends, SSE moves, even segment moves); every
/// `in`/`out`-form likewise (`int*` was vetoed above — and `inc*` rides
/// this rule, NOT the clobber set: `inc`/`dec` preserve CF); plain
/// pushes/pops only move data (`pushf*`/`popf*` were classified above);
/// non-CF `cmov`/`set`/`j` conditions read only ZF/SF/OF/PF (their CF
/// siblings were vetoed above) and an odd-case `jmp` reads nothing.
fn cf_skip_prefix(tok: &str) -> bool {
    let starts_with_fold =
        |lit: &str| tok.len() >= lit.len() && tok[..lit.len()].eq_ignore_ascii_case(lit);
    starts_with_fold("mov")
        || starts_with_fold("in")
        || starts_with_fold("out")
        || starts_with_fold("push")
        || starts_with_fold("pop")
        || starts_with_fold("cmov")
        || starts_with_fold("set")
        || starts_with_fold("j")
        || starts_with_fold("prefetch")
        || starts_with_fold("clflush")
        || tok.eq_ignore_ascii_case("mfence")
        || tok.eq_ignore_ascii_case("lfence")
        || tok.eq_ignore_ascii_case("sfence")
        || tok.eq_ignore_ascii_case("clwb")
}

/// The flag-effect token of a line: the mnemonic with any repeat/lock
/// prefix stripped (`rep cmpsb` writes flags, `rep movsq` does not).
fn cf_effect_token(t: &str) -> &str {
    let mut words = t.split_whitespace();
    let first = words.next().unwrap_or("");
    if first.eq_ignore_ascii_case("rep")
        || first.eq_ignore_ascii_case("repe")
        || first.eq_ignore_ascii_case("repne")
        || first.eq_ignore_ascii_case("repz")
        || first.eq_ignore_ascii_case("repnz")
        || first.eq_ignore_ascii_case("lock")
    {
        words.next().unwrap_or("")
    } else {
        first
    }
}

/// Whether a guarded shift/rotate (see `SHIFTS_GUARDED`) provably writes CF:
/// the count must be a `$`-immediate whose masked value is nonzero (silicon
/// masks to 5 bits, 6 for qword; a zero effective count preserves ALL
/// flags). A `%cl` count or a suffixless mnemonic (width unknowable) proves
/// nothing — and that is safe, because shifts never READ CF.
fn shift_proves_cf_clobber(tok: &str, t: &str) -> bool {
    if tok.len() != 4 {
        return false; // suffixless (or malformed): width unknowable
    }
    let Some(rest) = t.split_whitespace().nth(1) else {
        return false;
    };
    let Some((count, _)) = split_two_operands(rest) else {
        return false;
    };
    let Some(imm) = parse_imm32(count) else {
        return false;
    };
    let mask = if tok.as_bytes()[3].eq_ignore_ascii_case(&b'q') {
        63u32
    } else {
        31u32
    };
    imm & mask != 0
}

/// Strip ONE trailing x86 size letter (`b`/`w`/`l`/`q`) so a suffixed form
/// can be matched against a base-mnemonic table. Length-guarded at 2 so `bt`,
/// `or`, `jl` and `bt`-family short names are never truncated into a base.
fn strip_one_size_letter(tok: &str) -> Option<&str> {
    let b = tok.as_bytes();
    let last = *b.last()?;
    if b.len() > 2 && matches!(last, b'b' | b'w' | b'l' | b'q') {
        Some(&tok[..b.len() - 1])
    } else {
        None
    }
}

/// Whether `tok` provably writes OF (see `OF_CLOBBER_BASE` for the contract
/// and for why suffix stripping is safe on the credit side only).
fn writes_of(tok: &str) -> bool {
    let hit = |lit: &str| tok.eq_ignore_ascii_case(lit);
    OF_CLOBBER_BASE.iter().any(|lit| hit(lit))
        || strip_one_size_letter(tok).is_some_and(|base| {
            OF_CLOBBER_BASE
                .iter()
                .any(|lit| base.eq_ignore_ascii_case(lit))
        })
}

/// P2.5's flag-compensation audit: `true` when the consumer at line `j` may
/// fold cross-family. The scan below the consumer succeeds on the first
/// flag-clobbering instruction (whose own inputs — hence its CF — are
/// identical in both spellings) and vetoes on any carry-flag reader first.
/// Calls and returns kill flag knowledge (LLVM models no flag liveness
/// across either: the callee observes scratch flags and defines nothing;
/// flags at `ret` are undefined). Every other barrier ends the knowable
/// region with unknown readers past it. Renames the pass applies later
/// never change opcodes, so this pre-rename scan stays valid. Default-deny:
/// an unclassified mnemonic (all of SSE/AVX/x87, future ISA, typos in
/// hand-written asm) vetoes — over-refusal costs folds, never correctness.
fn shift_flags_safe_after(store: &LineStore, infos: &[LineInfo], j: usize, len: usize) -> bool {
    // CF and OF are tracked independently: each becomes dead at the first
    // instruction that overwrites it, and the fold stands only once BOTH are
    // dead before any reader of either can observe them. A flag that is
    // already dead cannot be vetoed by a later reader — that reader sees the
    // overwriting instruction's flag, identical in both spellings because its
    // own inputs are.
    let mut cf_dead = false;
    let mut of_dead = false;
    let mut n = j + 1;
    while n < len {
        if infos[n].is_nop() {
            n += 1;
            continue;
        }
        if infos[n].pinned {
            return false;
        }
        // A call or ret ends the knowable region with carry dead: the callee
        // is unknown code (LLVM-parity), and flags at `ret` are undefined
        // (async-signal/trap-frame flag observation is out of model).
        if matches!(infos[n].kind, LineKind::Call | LineKind::Ret) {
            return true;
        }
        if infos[n].is_barrier() {
            return false;
        }
        let t = infos[n].trimmed(store.get(n));
        let tok = cf_effect_token(t);
        // Readers veto only while the flag they read is still live.
        if !cf_dead && CF_READERS.iter().any(|lit| tok.eq_ignore_ascii_case(lit)) {
            return false;
        }
        if !of_dead && OF_READERS.iter().any(|lit| tok.eq_ignore_ascii_case(lit)) {
            return false;
        }
        let guarded_shift = SHIFTS_GUARDED
            .iter()
            .any(|lit| tok.eq_ignore_ascii_case(lit));
        let credits_cf = CF_CLOBBER.iter().any(|lit| tok.eq_ignore_ascii_case(lit))
            || (guarded_shift && shift_proves_cf_clobber(tok, t));
        let credits_of = writes_of(tok);
        if credits_cf {
            cf_dead = true;
        }
        if credits_of {
            of_dead = true;
        }
        if cf_dead && of_dead {
            return true;
        }
        // PARTIAL credit must keep the scan alive: an OF-blind CF writer
        // (`clc`, `stc`, `sahf`, `bt*`) ends the CF question but leaves OF
        // divergent, and falling through to default-deny here would refuse
        // every shape whose first clobber happens to be one of them.
        //
        // This branch runs BEFORE default-deny, so it is sound only because
        // both credit sets are explicit whitelists: matching one IS the
        // classification of the line. Widening `writes_of` with a prefix rule
        // (the way `cf_skip_prefix` works) would silently turn default-deny
        // into continue for unclassified mnemonics. Mutation-verified: forcing
        // `writes_of` to `true` and blanking `OF_READERS` fails the three OF
        // refusals below AND
        // `rmw_coalesce_refuses_movl_shlq32_before_unknown_mnemonic`, which is
        // the guard that catches exactly that widening.
        if credits_cf || credits_of {
            n += 1;
            continue;
        }
        if CF_SKIP_EXACT
            .iter()
            .any(|lit| tok.eq_ignore_ascii_case(lit))
        {
            n += 1;
            continue;
        }
        if cf_skip_prefix(tok) {
            n += 1;
            continue;
        }
        if guarded_shift {
            // A shift never READS CF, and a `%cl`/suffixless count proves no
            // clobber: skipping loses credit but stays sound.
            n += 1;
            continue;
        }
        return false;
    }
    false
}

/// Parse a `$`-immediate operand into its 64-bit value (for `movabsq`).
/// Mirrors `parse_imm32` (whose `u32` cannot hold a 64-bit address).
fn parse_imm64(src: &str) -> Option<u64> {
    let digits = src.strip_prefix('$')?;
    if digits.is_empty() {
        return None;
    }
    if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(neg) = digits.strip_prefix('-') {
        neg.parse::<i64>().ok().map(|v| (-v) as u64)
    } else {
        digits.parse::<u64>().ok()
    }
}

/// The destination operand's text of `t` (past the mnemonic): the last
/// top-level operand, or the sole operand when there is no comma.
fn dest_operand_text(t: &str) -> &str {
    let rest = t.find(' ').map(|i| t[i + 1..].trim_start()).unwrap_or("");
    match last_top_level_comma(rest.as_bytes()) {
        Some(c) => rest[c + 1..].trim(),
        None => rest.trim(),
    }
}

/// Whether line `t` (known to write family `fam`) establishes a zero upper
/// half: a 32-bit-destination write (EVERY dword dest zero-extends —
/// `movl`/`andl`/`cmovl`/3-operand `imull`/`leal`/`btsl`/`popcntl`/`incl`/
/// ... — the mnemonic is irrelevant), a zeroing idiom (`xorq %S, %S` /
/// `subq %S, %S`; `xorl`-self is subsumed by the dword rule), or a small
/// immediate (`movq $imm`: GAS sign-extends imm32, so only nonnegative
/// counts — `$0xFFFFFFFF` is -1, NOT small; `movabsq $imm64`: only low
/// 32 bits set). Callers guarantee lowercase text (see `upper32_zero_at`).
fn s_write_proves_upper_zero(t: &str, fam: RegId) -> bool {
    if dest_operand_text(t) == REG_NAMES[1][fam as usize] {
        // ...except conditional writers: `cmov*`/`cfcmov*`/`cmpxchg*`/`bsf`/`bsr`/
        // `rdrand`/`rdseed` leave the dest unchanged on the untaken/failed
        // path, so the old (unknown) upper half may survive (cmpxchg also
        // stops the scan first via the shared implicit-GP flag — both layers
        // refuse, by design). Likewise refuse explicit size/address prefixes
        // (hand-asm-only; GAS never emits them): the textual dword may not
        // match the silicon width. The token strips `lock`/`rep`
        // (width-transparent) before the check.
        let tok = cf_effect_token(t);
        if tok == "data16"
            || tok == "data32"
            || tok == "o16"
            || tok == "o32"
            || tok == "addr32"
            || tok == "adsize"
            || tok == "a16"
            || tok == "a32"
            || tok == "rex"
            || tok == "rex64"
            || tok == "rexw"
            || tok.starts_with("cmov")
            || tok.starts_with("cfcmov")
            || tok.starts_with("cmpxchg")
            || matches!(tok, "bsf" | "bsfl" | "bsr" | "bsrl" | "rdrand" | "rdseed")
        {
            return false;
        }
        return true;
    }
    let mnemonic = t.split_whitespace().next().unwrap_or("");
    let rest = t[mnemonic.len()..].trim_start();
    let qword = REG_NAMES[0][fam as usize];
    if mnemonic == "xorq" || mnemonic == "subq" {
        if let Some((s0, s1)) = split_two_operands(rest) {
            return s0 == qword && s1 == qword;
        }
        return false;
    }
    if mnemonic == "movq" {
        if let Some((imm, dst)) = split_two_operands(rest) {
            if dst == qword {
                return parse_imm32(imm).is_some_and(|v| (v as i32) >= 0);
            }
        }
        return false;
    }
    if mnemonic == "movabs" || mnemonic == "movabsq" {
        if let Some((imm, dst)) = split_two_operands(rest) {
            if dst == qword {
                return parse_imm64(imm).is_some_and(|v| v <= 0xFFFF_FFFF);
            }
        }
        return false;
    }
    false
}

/// P3's zero-upper proof: `true` when family `s_fam` provably holds a zero
/// upper half at line `li` (exclusive). Scans back within the block for the
/// NEAREST S-family write: a proving write (see `s_write_proves_upper_zero`)
/// proves it; any other write — a wider write, a partial write — refuses,
/// as does reaching a barrier/call/pinned line, an implicit-register line,
/// or an odd-case line first. Entry values (parameters) carry garbage upper
/// halves per SysV, so "no definer found" correctly refuses; reads of S
/// between the definer and the copy are fine (they change nothing). The
/// uniform stops (calls even for callee-saved S, implicit lines even for
/// other families) are deliberate: the foregone shapes are unmeasured and
/// the audit stays obviously sound.
pub(super) fn upper32_zero_at(
    store: &LineStore,
    infos: &[LineInfo],
    li: usize,
    s_fam: RegId,
) -> bool {
    if li == 0 {
        return false;
    }
    let mask = 1u16 << s_fam;
    let mut n = li;
    while n > 0 {
        n -= 1;
        if infos[n].is_nop() {
            continue;
        }
        // (`ret` needs no stop here: a fold below an earlier `ret` rewrites
        // dead code, which is effect-free.)
        if infos[n].pinned || infos[n].is_barrier() {
            return false;
        }
        let t = infos[n].trimmed(store.get(n));
        // The reference scan below is case-sensitive: refuse odd-case text
        // rather than risk skipping an unseen S-family write and proving
        // from a stale definer.
        if t.bytes().any(|b| b.is_ascii_uppercase()) {
            return false;
        }
        // `lock` is width- and value-transparent for the proof (and the fold
        // never touches the definer: it sits strictly above the copy by index
        // ordering, so the locked op stays bit-identical and its atomicity is
        // preserved) — strip it and evaluate the remainder.
        let t = t.strip_prefix("lock ").unwrap_or(t);
        if has_implicit_reg_usage(t) {
            return false;
        }
        if infos[n].reg_refs & mask == 0 {
            continue;
        }
        if !writes_family(&infos[n], t, s_fam) {
            continue; // pure read: the value flows through unchanged
        }
        return s_write_proves_upper_zero(t, s_fam);
    }
    false
}

/// `true` when `t` is a two-operand ALU read-modify-write fully redefining
/// `fam`: the destination names `fam` at 32/64 bits and the mnemonic reads it
/// (add/sub/logic/imul-2op/shift — every one of these reads its destination,
/// so with `%S` holding `%D`'s current value the renamed op computes the
/// identical result+flags into `%S`). The three-operand `imul` form is a pure
/// write (no dest read); one-operand forms, `adc`/`sbb`, partial writes,
/// `cmov`/`xchg` and port I/O are all conservatively excluded — unmeasured,
/// kept aborting.
fn is_rmw_of_family(t: &str, fam: RegId) -> bool {
    if !dest_is_full_width(t, fam) {
        return false;
    }
    let trimmed = t.trim_start();
    let Some(sp) = trimmed.find(' ') else {
        return false;
    };
    if !matches!(
        &trimmed[..sp],
        "andl"
            | "orl"
            | "xorl"
            | "addl"
            | "subl"
            | "imull"
            | "shll"
            | "shrl"
            | "sarl"
            | "sall"
            | "andq"
            | "orq"
            | "xorq"
            | "addq"
            | "subq"
            | "imulq"
            | "shlq"
            | "shrq"
            | "sarq"
            | "salq"
    ) {
        return false;
    }
    let Some((s0, _)) = split_two_operands(trimmed[sp + 1..].trim_start()) else {
        return false;
    };
    // A top-level comma inside the first operand means three operands (the
    // pure-write `imul` form); commas inside memory parens are fine.
    last_top_level_comma(s0.as_bytes()).is_none()
}

pub(super) fn coalesce_copy_into_rmw(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        let li = i;
        i += 1;
        if infos[li].is_nop() || infos[li].pinned || infos[li].is_barrier() {
            continue;
        }
        let mov = infos[li].trimmed(store.get(li)).to_string();
        // A plain copy (`movl`/`movq`), or a zero/sign-extending copy whose
        // extension garbage the consumer's mask provably kills: `movzbl %dl,
        // %r11d; andl $127, %r11d` ≡ `movl %edx, %r11d; andl $127, %r11d`,
        // since the mask clears every bit the two spellings could disagree
        // on. `ext_bits` records how many low bits they agree on.
        let (copy_w, copy_rest, ext_bits) = if let Some(r) = mov.strip_prefix("movq ") {
            (64u8, r, None)
        } else if let Some(r) = mov.strip_prefix("movl ") {
            (32u8, r, None)
        } else if let Some(r) = mov
            .strip_prefix("movzbl ")
            .or_else(|| mov.strip_prefix("movsbl "))
        {
            (32u8, r, Some(8u32))
        } else if let Some(r) = mov
            .strip_prefix("movzwl ")
            .or_else(|| mov.strip_prefix("movswl "))
        {
            (32u8, r, Some(16u32))
        } else {
            continue;
        };
        let Some((copy_src, copy_dst)) = split_two_operands(copy_rest) else {
            continue;
        };
        let (Some(s_fam), Some(d_fam)) = (plain_gp_operand(copy_src), plain_gp_operand(copy_dst))
        else {
            continue;
        };
        if s_fam == d_fam || !is_relayable_family(s_fam) || !is_relayable_family(d_fam) {
            continue;
        }
        // An extension copy must read the NARROW source at exactly the
        // extension width and write the 32-bit destination: anything else is
        // either invalid x86 or not an extension at all.
        if let Some(bits) = ext_bits {
            let narrow = if bits == 8 { 3usize } else { 2usize };
            if copy_src != REG_NAMES[narrow][s_fam as usize]
                || copy_dst != REG_NAMES[1][d_fam as usize]
            {
                continue;
            }
        }
        // The consumer: adjacent (NOPs apart) so `%S == %D` still holds, a
        // two-operand ALU op destinationing exactly `%D` at a width the copy
        // established.
        let mut j = li + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len || infos[j].pinned || infos[j].is_barrier() {
            continue;
        }
        let cons = infos[j].trimmed(store.get(j)).to_string();
        if has_implicit_reg_usage(&cons) {
            continue;
        }
        let Some(op) = RMW_CONSUMER_OPS.iter().find(|op| cons.starts_with(**op)) else {
            continue;
        };
        let Some((cons_src, cons_dst)) = split_two_operands(&cons[op.len()..]) else {
            continue;
        };
        // Extension copies need an `andl $mask` consumer whose mask kills
        // every bit above the extension width: below it both spellings agree
        // (the extension's identity bits), above it the mask forces zero in
        // both. Any other consumer — or an unparseable/wider mask — refuses.
        if let Some(bits) = ext_bits {
            if *op != "andl " {
                continue;
            }
            let Some(mask) = parse_imm32(cons_src) else {
                continue;
            };
            if mask & !((1u32 << bits) - 1) != 0 {
                continue;
            }
        }
        let (Some(cons_dst_fam), Some(cons_w)) = (plain_gp_operand(cons_dst), dest_width(cons_dst))
        else {
            continue;
        };
        if cons_dst_fam != d_fam {
            continue;
        }
        // P2.5 cross-family exception: `movl %S, %D` + `shlq $32, %D`
        // computes the identical 64-bit value into `%S` — unfolded,
        // `%D = (u64)%S_lo32 << 32`; folded, `%S <<= 32`, and the shift
        // destroys exactly the bits the zero-extension had fixed (the old
        // upper half shifts out, the low half fills with zero). CF
        // diverges (0 vs the source's bit 32), so the fold additionally
        // needs `shift_flags_safe_after`. The count check masks to 6 bits:
        // silicon shifts by `imm & 63`, so a naive `imm >= 32` test would
        // miscompile `shlq $64` (a shift by zero: unfolded `%D` is
        // zero-extended, folded `%S` is not).
        // P3 zero-upper exception: a 64-bit consumer under a 32-bit copy
        // also folds when the source's upper half is PROVEN zero at the
        // copy (`upper32_zero_at`) — then `%S64 ≡ %D64` exactly and the
        // consumer computes identical values AND flags (no audit needed).
        if cons_w > copy_w
            && !(copy_w == 32
                && ext_bits.is_none()
                && *op == "shlq "
                && parse_imm32(cons_src).is_some_and(|c| c & 63 == 32)
                && shift_flags_safe_after(store, infos, j, len))
            && !(copy_w == 32 && ext_bits.is_none() && upper32_zero_at(store, infos, li, s_fam))
        {
            continue;
        }
        // The renamer doubles as the consumer validator: a plain-register
        // `%D` source (`addl %D, %D`) retargets soundly (both operands hold
        // the same value), while a memory mention of `%D` refuses the
        // candidate. The rollback original is the RAW stored line: restoring
        // the trimmed matcher would strip the line's indentation on the
        // refuse path.
        let Some(new_cons) = rename_plain_family_reads(&cons, d_fam, s_fam) else {
            continue;
        };
        let orig_cons = store.get(j).to_string();
        // The consumer's write unconditionally clobbers the source family,
        // so the source's OLD value must be dead at the copy: dataflow
        // (loop-aware, RET_LIVE-aware) or barrier-conservative block-local
        // reasoning. Whole-function textual uniqueness additionally justifies
        // the NEW write to %S ONLY for caller-saved %S outside loops: (a) in
        // a loop the copy re-executes and iteration N+1 rereads %S even
        // though %S is mentioned only at the copy (torture
        // execute/builtin-bitops-1.c: `movq %rdi,%r9; andq %r8,%r9` with
        // loop-invariant %rdi folded into `andq %r8,%rdi`, corrupting every
        // iteration past the first); (b) a callee-saved %S is owed its entry
        // value at every `ret`, an obligation no text scan can see.
        // Invisible back-edges (a libc longjmp landing above the copy) need
        // no extra arm: they ride on `call`, calls clobber caller-saved %S
        // and force a mentioning redefinition between the call and the copy,
        // which already fails uniqueness — the textual loop scan is complete
        // for caller-saved sources.
        if !(matches!(lv.live_after(li, s_fam), Some(false))
            || dead_in_block_after(store, infos, li + 1, s_fam)
            || (is_caller_saved_family(s_fam)
                && !copy_in_loop(&lv, store, infos, li)
                && family_private_lv(&lv, store, infos, li, s_fam, &[li])))
        {
            continue;
        }
        // --- rename window: plain-register reads of %D until the first full
        // --- redefinition of %D or a barrier. Collect rewrites; abort on
        // --- anything unrenamable.
        let d_mask = 1u16 << d_fam;
        let mut rewrites: Vec<(usize, String, String)> = vec![(j, orig_cons, new_cons)];
        let mut abort = false;
        // Whether the coalesced value's most recent write was a dword write
        // (upper half zero): a dword copy back into %S is then the identity.
        let mut upper_zero = cons_w == 32;
        // A copy back into the source that the rewrite deletes.
        let mut dropped_copy_back: Option<usize> = None;
        let mut k = j + 1;
        while k < len {
            if infos[k].is_nop() {
                k += 1;
                continue;
            }
            if infos[k].pinned || has_implicit_reg_usage(infos[k].trimmed(store.get(k))) {
                abort = true;
                break;
            }
            if infos[k].is_barrier() {
                break; // downstream paths are the deadness proof's job
            }
            let t = infos[k].trimmed(store.get(k)).to_string();
            // The copy BACK into the source (`movl %D, %S` / `movq %D, %S`;
            // the call-result relay `movl %eax,%r11d; addl M,%r11d; movl
            // %r11d,%eax`): %S already holds %D's full value, so the qword
            // copy is the identity, and so is the zero-extending dword copy
            // when the last write of the value was a dword write; otherwise
            // it stays as `movl %S, %S`. The window ends here; a later %D read
            // fails the deadness proof below and rolls everything back.
            if let Some(qword) = copy_back_width(&t, d_fam, s_fam) {
                if qword || upper_zero {
                    dropped_copy_back = Some(k);
                } else {
                    let s32 = REG_NAMES[1][s_fam as usize];
                    rewrites.push((
                        k,
                        store.get(k).to_string(),
                        format!("    movl {s32}, {s32}"),
                    ));
                }
                break;
            }
            if line_refs_family(&t, s_fam) {
                abort = true;
                break;
            }
            if infos[k].reg_refs & d_mask == 0 {
                k += 1;
                continue;
            }
            // %D redefined without reading itself: later reads see the new
            // def, not our value — the window ends.
            if is_full_write(&infos[k], &t, d_fam) {
                break;
            }
            // A read-modify-write of %D renames like a read: %S holds %D's
            // current value (the retargeted consumer established it and every
            // prior rename preserved it), so the renamed op computes the
            // identical result and flags from the same values into %S, and
            // later %D reads (renamed to %S) observe exactly the value they
            // would have. Pure/partial writes (`movb`, `setcc`), `cmov` and
            // `xchg` still abort (unmeasured, conservatively kept), as does
            // anything the renamer cannot spell (memory mentions of %D, a
            // `%cl` count of the coalesced family).
            if writes_family(&infos[k], &t, d_fam) {
                if !is_rmw_of_family(&t, d_fam) {
                    abort = true;
                    break;
                }
                let Some(new_t) = rename_plain_family_reads(&t, d_fam, s_fam) else {
                    abort = true;
                    break;
                };
                upper_zero = dest_width(dest_operand_text(&t)) == Some(32);
                rewrites.push((k, store.get(k).to_string(), new_t));
                k += 1;
                continue;
            }
            let Some(new_t) = rename_plain_family_reads(&t, d_fam, s_fam) else {
                abort = true;
                break;
            };
            // Raw stored line (see the consumer note above): a trimmed
            // rollback original would strip indentation on the refuse path.
            rewrites.push((k, store.get(k).to_string(), new_t));
            k += 1;
        }
        if abort {
            continue;
        }

        // --- apply, prove, or roll back (same discipline as
        // --- `fold_copy_into_lea_base`) -----------------------------------
        let orig_copy = store.get(li).to_string();
        mark_nop(&mut infos[li]);
        for &(idx, _, ref new_t) in &rewrites {
            replace_line(store, &mut infos[idx], idx, new_t.clone());
        }
        let orig_copy_back = dropped_copy_back.map(|idx| {
            let orig = store.get(idx).to_string();
            mark_nop(&mut infos[idx]);
            (idx, orig)
        });
        // The proof needs the solution of the REWRITTEN text.  Every edit
        // lies in `li..=last` (the copy, the consumer at `j > li`, and the
        // renamed reads / dropped copy-back after it), so a splice of a copy
        // of `lv` over that span is that solution (`refresh_span` falls back
        // to re-solving the one function when the span is not straight-line).
        // Re-solving the whole file per candidate made this pass 3% of
        // compiling zstd's huf_compress.c.
        let last = rewrites
            .iter()
            .map(|r| r.0)
            .chain(orig_copy_back.as_ref().map(|c| c.0))
            .fold(j, usize::max);
        let mut lv2 = lv.clone();
        lv2.refresh_span(store, infos, li, last);
        // The deadness query is anchored at the consumer line, NOT at the
        // copy: the copy is NOP-marked at this point and `FileLiveness` only
        // marks real instructions as known, so a query at its index would
        // answer `None` and silently degrade the proof to its syntactic
        // fallbacks. Nothing between the copy and the consumer touches `%D`,
        // so "dead after the consumer" is exactly "dead after the (deleted)
        // copy" on the rewritten text.
        if provably_dead_lv(&lv2, store, infos, j, d_fam, &[li, j]) {
            lv = lv2;
            changed = true;
            i = j + 1;
        } else {
            replace_line(store, &mut infos[li], li, orig_copy);
            for (idx, orig, _) in rewrites {
                replace_line(store, &mut infos[idx], idx, orig);
            }
            if let Some((idx, orig)) = orig_copy_back {
                replace_line(store, &mut infos[idx], idx, orig);
            }
            // `lv` still describes the restored text.
        }
    }
    changed
}

/// `Some(is_qword)` when `t` is exactly the full-width copy `mov{l,q} %D, %S`
/// from family `d` into family `s` (lowercase allocator spelling).
fn copy_back_width(t: &str, d: RegId, s: RegId) -> Option<bool> {
    let rest = t
        .strip_prefix("movq ")
        .map(|r| (true, r))
        .or_else(|| t.strip_prefix("movl ").map(|r| (false, r)));
    let (qword, rest) = rest?;
    let w = if qword { 0 } else { 1 };
    let (src, dst) = split_two_operands(rest)?;
    (src == REG_NAMES[w][d as usize] && dst == REG_NAMES[w][s as usize]).then_some(qword)
}

/// Rename every PLAIN-REGISTER mention of family `from` to the same-width
/// register of family `to` inside one instruction's text. Returns `None`
/// when `from` appears anywhere this routine cannot rewrite (a memory
/// operand base/index, or an unknown width spelling), so the caller can
/// abort conservatively. Valid for read-only mentions and for `cmp`/`test`
/// (whose "destination" is flags); callers must have excluded RMW and
/// full-write lines before calling.
fn rename_plain_family_reads(t: &str, from: RegId, to: RegId) -> Option<String> {
    let trimmed = t.trim_start();
    let sp = trimmed.find(' ')?;
    let (mnemonic, rest) = trimmed.split_at(sp);
    let rest = rest.trim_start();
    let (s0, s1) = split_two_operands(rest)?;
    // The shift/rotate count register is FIXED by the ISA: only `%cl` is
    // encodable there, so a `%cl` source on these mnemonics can neither be
    // renamed (invalid x86) nor kept (it would read the stale pre-rewrite
    // value). Refuse; both callers abort/roll back on `None`. `%cl` as an
    // ordinary byte operand (`movzbl %cl, %eax`) still renames — any byte
    // register is a valid, value-identical spelling there.
    let count_fixed = matches!(
        mnemonic,
        "shl"
            | "shlb"
            | "shlw"
            | "shll"
            | "shlq"
            | "sal"
            | "salb"
            | "salw"
            | "sall"
            | "salq"
            | "shr"
            | "shrb"
            | "shrw"
            | "shrl"
            | "shrq"
            | "sar"
            | "sarb"
            | "sarw"
            | "sarl"
            | "sarq"
            | "rol"
            | "rolb"
            | "rolw"
            | "roll"
            | "rolq"
            | "ror"
            | "rorb"
            | "rorw"
            | "rorl"
            | "rorq"
            | "rcl"
            | "rclb"
            | "rclw"
            | "rcll"
            | "rclq"
            | "rcr"
            | "rcrb"
            | "rcrw"
            | "rcrl"
            | "rcrq"
    );
    let ren = |op: &str| -> Option<String> {
        if let Some(f) = plain_gp_operand(op) {
            if f != from {
                return Some(op.to_string());
            }
            if count_fixed && op == "%cl" {
                return None; // unencodable anywhere else; unkeepable (stale)
            }
            for row in REG_NAMES.iter() {
                if row[from as usize] == op {
                    return Some(row[to as usize].to_string());
                }
            }
            return None; // unknown width spelling
        }
        if line_refs_family(op, from) {
            return None; // memory mention — not renamable here
        }
        Some(op.to_string())
    };
    let n0 = ren(s0)?;
    let n1 = ren(s1)?;
    Some(format!("    {} {}, {}", mnemonic, n0, n1))
}

// ── Scaled-index temporary into a memory operand ────────────────────────────
//
// Address operands that never reach the codegen-time GEP folds (the
// `__builtin_prefetch` pointer, explicit-SIMD intrinsic pointers) arrive with
// their scaled index materialised in a temporary. The measured shapes, from
// the canonical software-prefetch loop `__builtin_prefetch(&a[i + K])`:
//
//     leaq 0(,%r9,8), %r11              leaq 8(%r8), %r9
//     prefetcht0 128(%rdi,%r11,1)       shlq $3, %r9
//                                       prefetcht0 (%rdi,%r9)
//
// `fold_lea_into_memory_op` splices only a temporary in BASE position and
// skips index-only producers, and `fold_shift_into_sib` needs an `addq`
// consumer, so both survive to the final assembly: one or two extra
// instructions per loop iteration against GCC's single
// `prefetcht0 128(%rdi,%r9,8)` / `prefetcht0 64(%rdi,%r8,8)`.
//
// The fold composes the producer into the consumer's operand where the
// temporary is the UNSCALED index (beside a base) or the sole base, through
// the same `compose_sib` every LEA fold uses (term merging, scale legality,
// disp32 range — all fail closed).
//
// Sound when:
//   * the producer is `leaq ADDR, %T` with an index-only ADDR, or a base-only
//     `leaq K(%I), %T` immediately followed by `shlq/salq $s, %T` (s = 1..3,
//     64-bit only: `shll` zero-extends, which `lea` scaling does not model),
//     whose deleted flag write must be dead (`flags_dead_after`);
//   * the consumer immediately follows, mentions `%T`'s family exactly once —
//     inside one memory operand, as `D(%B,%T[,1])` or `D(%T)` — has no
//     implicit use of it, and `%T` is dead after it. The producer's inputs
//     cannot change in between (adjacency), and with the producer deleted
//     `%T` keeps its pre-producer value, so `%I == %T` composes correctly;
//   * a `lea` consumer composes to a two-component `(%B,%I,s)` address with
//     no displacement or symbol (a base+index+disp LEA is the 3-cycle "slow
//     LEA" on Skylake-family cores, so turning two 1-cycle LEAs into one is
//     not a clear win there; a memory access gets the third component free in
//     the AGU); the consumer has no `*` (indirect branch) and no segment
//     override.
pub(super) fn fold_scaled_index_temp_into_mem(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut dead = LazyDeadness::new();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if debug_transparent(store, infos[i], i) || infos[i].pinned || infos[i].is_barrier() {
            i += 1;
            continue;
        }
        let Some((producer, dst, shl)) = scaled_temp_producer(store, infos, i, len) else {
            i += 1;
            continue;
        };
        let j = next_non_debug(store, infos, shl.unwrap_or(i) + 1, len);
        if j >= len || infos[j].is_barrier() || infos[j].pinned {
            i += 1;
            continue;
        }
        let dst_fam = register_family_fast(&dst);
        let consumer = infos[j].trimmed(store.get(j));
        if implicit_reg_refs(consumer.as_bytes()) & (1u16 << dst_fam) != 0 {
            i += 1;
            continue;
        }
        let Some(replacement) = splice_temp_into_mem_operand(consumer, &dst, dst_fam, &producer)
        else {
            i += 1;
            continue;
        };
        let owned = [i, shl.unwrap_or(i), j];
        if !dead.dead_after(store, infos, j, dst_fam, &owned) {
            i += 1;
            continue;
        }
        mark_nop(&mut infos[i]);
        if let Some(k) = shl {
            mark_nop(&mut infos[k]);
        }
        replace_line(store, &mut infos[j], j, format!("    {replacement}"));
        dead.invalidate(store, infos, j);
        changed = true;
        i = j + 1;
    }
    changed
}

/// The producer of a scaled-index temporary at line `i`: its address, the
/// 64-bit temporary, and the `shl` line for the two-instruction shape (see
/// [`fold_scaled_index_temp_into_mem`]).
fn scaled_temp_producer(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    len: usize,
) -> Option<(SibAddr, String, Option<usize>)> {
    let t = infos[i].trimmed(store.get(i));
    let (addr_text, dst) = t.strip_prefix("leaq ")?.rsplit_once(',')?;
    let (addr_text, dst) = (addr_text.trim(), dst.trim());
    let fam = register_family_fast(dst);
    if fam == REG_NONE || fam > REG_GP_MAX || REG_NAMES[0][fam as usize] != dst {
        return None;
    }
    if addr_text.contains(':') {
        return None;
    }
    let addr = SibAddr::parse(addr_text)?;
    if addr.base.is_none() && addr.index.is_some() {
        return Some((addr, dst.to_string(), None));
    }
    let base = match (&addr.base, &addr.index, &addr.sym) {
        (Some(b), None, None) => b.clone(),
        _ => return None,
    };
    let k = next_non_debug(store, infos, i + 1, len);
    if k >= len || infos[k].pinned || infos[k].is_barrier() {
        return None;
    }
    let shl = infos[k].trimmed(store.get(k));
    let (mnemonic, rest) = shl.split_once(' ')?;
    if !matches!(mnemonic, "shlq" | "salq") {
        return None;
    }
    let (imm, reg) = split_two_operands(rest.trim_start())?;
    let s: u32 = imm.strip_prefix('$')?.parse().ok()?;
    if reg != dst || !(1..=3).contains(&s) {
        return None;
    }
    if !super::flag_peepholes::flags_dead_after(store, infos, k + 1) {
        return None;
    }
    let disp = addr.disp.checked_mul(1i64 << s)?;
    i32::try_from(disp).ok()?;
    Some((
        SibAddr {
            sym: None,
            disp,
            base: None,
            index: Some((base, 1u8 << s)),
        },
        dst.to_string(),
        Some(k),
    ))
}

/// Mentions of GP family `fam` in `line`, at every width (high byte too).
fn gp_family_mentions(line: &str, fam: RegId) -> usize {
    let fam_u = fam as usize;
    if fam_u >= REG_NAMES[0].len() {
        return usize::MAX;
    }
    let high = (fam_u < 4).then(|| super::helpers::HIGH_BYTE_NAMES[fam_u]);
    REG_NAMES
        .iter()
        .map(|row| row[fam_u])
        .chain(high)
        .map(|name| {
            line.match_indices(name)
                .filter(|(pos, _)| {
                    line.as_bytes()
                        .get(pos + name.len())
                        .is_none_or(|ch| !ch.is_ascii_alphanumeric())
                })
                .count()
        })
        .sum()
}

/// Rewrite the single memory operand of `line` that holds `%T` (`dst`) as
/// its unscaled index or sole base, composing `producer` into it. `None`
/// when the shape is anything else (see [`fold_scaled_index_temp_into_mem`]).
fn splice_temp_into_mem_operand(
    line: &str,
    dst: &str,
    dst_fam: RegId,
    producer: &SibAddr,
) -> Option<String> {
    let is_lea = line.starts_with("lea");
    if line.contains('*')
        || segment_prefix_len_anywhere(line)
        || gp_family_mentions(line, dst_fam) != 1
    {
        return None;
    }
    let pos = line
        .match_indices(dst)
        .find(|(p, _)| {
            line.as_bytes()
                .get(p + dst.len())
                .is_none_or(|ch| !ch.is_ascii_alphanumeric())
        })?
        .0;
    let open = line[..pos].rfind('(')?;
    if line[open..pos].contains(')') {
        return None;
    }
    let close = pos + line[pos..].find(')')?;
    let op_start = operand_start(line, open);
    let disp_text = line[op_start..open].trim();
    if disp_text.contains(['%', ':', '*', '(', ')', '$', ' ', '\t']) {
        return None;
    }
    let cons = SibAddr::parse(&format!("{}{}", disp_text, &line[open..=close]))?;
    let rest = match (&cons.base, &cons.index) {
        (Some(b), None) if b == dst => SibAddr {
            sym: cons.sym.clone(),
            disp: cons.disp,
            base: None,
            index: None,
        },
        (Some(b), Some((x, 1))) if x == dst && b != dst => SibAddr {
            sym: cons.sym.clone(),
            disp: cons.disp,
            base: Some(b.clone()),
            index: None,
        },
        // `(%T, %B, 1)`: at scale 1 base and index are interchangeable, and
        // `%B` moves to the base slot, where even `%rsp` is encodable.
        (Some(b), Some((x, 1))) if b == dst && x != dst => SibAddr {
            sym: cons.sym.clone(),
            disp: cons.disp,
            base: Some(x.clone()),
            index: None,
        },
        _ => return None,
    };
    // A `lea` consumer only when the composition is a two-component address
    // (`(%B, %I, s)`, no displacement, no symbol, no RIP base): that is one
    // 1-cycle LEA on Intel P-cores replacing two dependent ones (GCC emits
    // exactly `leaq (%r9, %r8, 8)` for `a + (b << 3)`); a displacement would
    // make it the 3-component slow LEA, which stays two instructions.
    if is_lea
        && (producer.sym.is_some()
            || rest.sym.is_some()
            || producer.disp.checked_add(rest.disp) != Some(0)
            || rest.base.as_deref() == Some("%rip"))
    {
        return None;
    }
    let composed = compose_sib(producer, &rest)?;
    let lead = line[op_start..open].len() - line[op_start..open].trim_start().len();
    let start = op_start + lead;
    Some(format!(
        "{}{}{}",
        &line[..start],
        composed,
        &line[close + 1..]
    ))
}

/// Any `%fs:`/`%gs:` (or other segment) override in `line`.
fn segment_prefix_len_anywhere(line: &str) -> bool {
    ["%fs:", "%gs:", "%es:", "%cs:", "%ss:", "%ds:"]
        .iter()
        .any(|seg| line.contains(seg))
}

#[cfg(test)]
mod tests {

    /// Whole-pipeline driver for the `%rbp` relay tests: a rewrite that is
    /// sound alone but misread by a neighbouring pass is still a bug.
    fn rbp_relay(body: &str) -> String {
        use super::super::super::peephole_optimize;
        peephole_optimize(format!(".text\nf:\n.cfi_startproc\n{body}\n.cfi_endproc\n"))
    }

    /// zstd_count's per-iteration zero-extension relay: `%rbp` is an ordinary
    /// allocator-owned register in an `%rsp`-relative frame (only the
    /// callee-save push/pop names it), the 32-bit `andl` proves its upper
    /// half zero, so the 64-bit read relays and the copy disappears. Before
    /// the `rbp_is_gpr_in_function` gate the identical `%r12` shape folded
    /// and this one did not.
    #[test]
    fn move_relay_reads_rbp_when_it_is_a_plain_gpr() {
        let out = rbp_relay(concat!(
            "    pushq %rbp\n",
            "    andl $1048319, %ebp\n",
            "    movl %ebp, %r11d\n",
            "    addq %r11, %rax\n",
            "    movl $0, %r11d\n",
            "    popq %rbp\n",
            "    ret",
        ));
        assert!(out.contains("addq %rbp, %rax"), "{out}");
        assert!(!out.contains("movl %ebp, %r11d"), "{out}");
    }

    /// Any parenthesised rbp operand proves the frame-pointer build: rbp is
    /// then part of the frame contract and the relay must stay.
    #[test]
    fn move_relay_leaves_rbp_alone_in_a_frame_pointer_function() {
        let out = rbp_relay(concat!(
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    movl -4(%rbp), %ecx\n",
            "    andl $1048319, %ebp\n",
            "    movl %ebp, %r11d\n",
            "    addq %r11, %rax\n",
            "    movl $0, %r11d\n",
            "    addl %ecx, %eax\n",
            "    popq %rbp\n",
            "    ret",
        ));
        assert!(out.contains("movl %ebp, %r11d"), "{out}");
        assert!(!out.contains("addq %rbp, %rax"), "{out}");
    }

    /// Even with rbp proven a GPR, the relay must not rename INTO an address:
    /// `(%rdi,%rbp)` would read as frame-slot traffic to every later pass and
    /// would itself be frame-pointer evidence, invalidating the proof.
    #[test]
    fn move_relay_never_creates_an_rbp_memory_operand() {
        let out = rbp_relay(concat!(
            "    pushq %rbp\n",
            "    andl $1048319, %ebp\n",
            "    movl %ebp, %r11d\n",
            "    movzbl (%rdi, %r11), %eax\n",
            "    movl $0, %r11d\n",
            "    popq %rbp\n",
            "    ret",
        ));
        assert!(!out.contains("%rbp)"), "{out}");
        assert!(!out.contains("(%rdi, %rbp"), "{out}");
    }

    /// `CfiIndex` answers exactly what the `function_range` scan answers, for
    /// every line: before the first function, inside one, on a marker,
    /// between two functions (an `endproc` nearer than any `startproc`),
    /// across a marker nopped before the index was built, inside a function
    /// left open at the end of the text, and after an `insert_line`.
    #[test]
    fn cfi_index_matches_function_range_scan() {
        let asm = concat!(
            "    .text\n",
            "f:\n.cfi_startproc\n",
            "    movl $1, %eax\n",
            "    ret\n.cfi_endproc\n",
            "    .p2align 4\n",
            "g:\n.cfi_startproc\n",
            "    .cfi_endproc\n", // nopped below: g continues past it
            "    movl $2, %eax\n",
            "    ret\n.cfi_endproc\n",
            "h:\n.cfi_startproc\n",
            "    movl $3, %eax\n",
            "    ret\n",
        );
        let mut store = super::LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        let stray = (0..store.len())
            .filter(|&i| store.get(i).trim() == ".cfi_endproc")
            .nth(1)
            .expect("fixture has a second endproc");
        super::mark_nop(&mut infos[stray]);

        let check = |store: &super::LineStore, infos: &[super::LineInfo], idx: &super::CfiIndex| {
            for i in 0..store.len() {
                assert_eq!(
                    idx.function_range(i, store.len()),
                    super::function_range(store, infos, i),
                    "line {i}: {:?}",
                    store.get(i)
                );
            }
        };
        let mut idx = super::CfiIndex::build(&store, &infos);
        check(&store, &infos, &idx);
        // Unterminated `h` runs to the end; the gap between f and g is in
        // no function.
        let last = store.len() - 1;
        assert_eq!(
            idx.function_range(last, store.len()).map(|r| r.1),
            Some(store.len())
        );
        let gap = (0..store.len())
            .find(|&i| store.get(i).trim() == ".p2align 4")
            .unwrap();
        assert_eq!(idx.function_range(gap, store.len()), None);

        // Insert inside g (and at the very front): the shifted index still
        // agrees with a fresh scan.
        for at in [stray + 2, 0] {
            let text = "    nop".to_string();
            store.insert_line(at, text.clone());
            infos.insert(at, super::classify_line(&text));
            idx.shift_for_insert(at);
            check(&store, &infos, &idx);
        }
    }

    #[test]
    fn mem_index_relay_folds_movl_copy() {
        // The unrolled histogram shape: a 32-bit relay copy feeding the SIB
        // index of an RMW, with the source zero-extended by a movzbl producer.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    movzbl (%rdi, %rsi), %eax\n",
            "    movl %eax, %r11d\n",
            "    leaq bins(%rip), %r9\n",
            "    addq $1, (%r9, %r11, 8)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert!(
            out.contains("addq $1, (%r9, %rax, 8)"),
            "the SIB index must relay to the zero-extended source:\n{out}"
        );
        assert!(!out.contains("%r11"), "the relay copy must be gone:\n{out}");
    }

    #[test]
    fn mem_base_relay_tolerates_dst_collision() {
        // `movq %rdx,%r9; movzbl (%rdi,%r9),%edx` -> `movzbl (%rdi,%rdx),%edx`:
        // the address is computed before the destination write.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    movq %rdx, %r9\n",
            "    movzbl (%rdi, %r9), %edx\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert!(
            out.contains("movzbl (%rdi, %rdx), %edx"),
            "the base relay must fold despite the dest collision:\n{out}"
        );
    }

    #[test]
    fn mem_relay_merges_same_value_sib_legally() {
        // `movq %r9,%r11` makes %r11 == %r9 at the consumer, so folding the
        // copy away and merging the SIB base/index to one register computes
        // %r9 + %r9*8 both before and after — the rename is value-preserving
        // even though the source appears twice in the line.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    movq %r9, %r11\n",
            "    addq $1, (%r11, %r9, 8)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert!(
            out.contains("addq $1, (%r9, %r9, 8)"),
            "the same-value SIB must fold to one register:\n{out}"
        );
        assert!(!out.contains("%r11"), "no stale %r11 may remain:\n{out}");
    }

    #[test]
    fn mem_relay_refuses_register_write_of_source() {
        // The consumer's bare-register destination is an RMW of the copy's
        // destination (`addl %esi, %r11d` reads %r11d first): a full rename
        // would write the copy's SOURCE register (`%eax`) and clobber its
        // live high half, and the source-only rename cannot touch an RMW
        // destination's read. The fold must refuse. The stored result keeps
        // the body observable (dead-code elimination must not decide this).
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    movq %rax, %r11\n",
            "    addl %esi, %r11d\n",
            "    movl %r11d, (%rdi)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        // The mem-relay contract this pins: whatever other passes make of the
        // pair (the lea former legitimately rebuilds it as
        // `leal (%rax,%rsi),%r11d`), the rename may never WRITE the copy's
        // source family — that would clobber its live high half.
        assert!(
            !out.contains(", %eax") && !out.contains(", %rax") && !out.contains(", %ax"),
            "the copy's source register must never be written:\n{out}"
        );
        assert!(
            out.contains("movl %r11d, (%rdi)"),
            "the stored result must stay observable:\n{out}"
        );
    }

    #[test]
    fn mem_relay_folds_write_only_reg_dest() {
        // The three-operand imul's destination is a WRITE-ONLY fresh value
        // (`imull %esi, (%r9,%r11,8), %r11d`): the SIB mentions of %r11 are
        // address reads that retarget to %rax, the destination register keeps
        // its own name, and the destination write kills the copy's value.
        // What used to be a refusal now folds to one instruction.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    movq %rax, %r11\n",
            "    imull %esi, (%r9, %r11, 8), %r11d\n",
            "    movl %r11d, (%rdi)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert!(
            out.contains("imull %esi, (%r9, %rax, 8), %r11d"),
            "the SIB retargets to %rax while the write-only dest stays:\n{out}"
        );
        assert!(
            !out.contains("movq %rax, %r11"),
            "the relayed copy must be gone:\n{out}"
        );
    }

    #[test]
    fn rip_lea_redef_is_deleted() {
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    leaq bins(%rip), %r9\n",
            "    addq $1, (%r9, %rax, 8)\n",
            "    leaq bins(%rip), %r9\n",
            "    addq $1, (%r9, %rdx, 8)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert_eq!(
            out.matches("leaq bins(%rip), %r9").count(),
            1,
            "the redundant redef must go:\n{out}"
        );
    }

    #[test]
    fn rip_lea_cse_rejects_location_relative_numeric_displacements() {
        // A numeric RIP-relative displacement is relative to each LEA's own
        // next instruction address. Reusing the first materialized value for
        // the second identical spelling would therefore change the address.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    leaq 8(%rip), %r9\n",
            "    movq %r9, (%rdi)\n",
            "    leaq 8(%rip), %r9\n",
            "    movq %r9, (%rsi)\n",
            "    ret\n.cfi_endproc\n",
        );
        let mut store = super::LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        assert!(
            !super::cse_symbol_lea(&mut store, &mut infos),
            "numeric RIP-relative expressions are position-dependent, not CSE-able"
        );
    }

    #[test]
    fn pr814_rip_lea_cse_preserves_current_location() {
        for expr in [".", ".+8", ".-8"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    leaq {expr}(%rip), %r9\n    movq %r9, (%rdi)\n    leaq {expr}(%rip), %r9\n    movq %r9, (%rsi)\n    ret\n.cfi_endproc\n"
            );
            let mut store = super::LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| super::classify_line(store.get(i)))
                .collect();
            assert!(
                !super::cse_symbol_lea(&mut store, &mut infos),
                "CSE of {expr}"
            );
        }
    }

    #[test]
    fn pr814_rip_lea_cse_preserves_pinned_materializations() {
        let asm = "f:\n.cfi_startproc\n    leaq bins(%rip), %r9\n    leaq bins(%rip), %r10\n    ret\n.cfi_endproc\n";
        let mut store = super::LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        infos[3].pinned = true;
        assert!(!super::cse_symbol_lea(&mut store, &mut infos));
        assert!(infos[3].pinned);
        assert_eq!(store.get(3).trim(), "leaq bins(%rip), %r10");
    }

    #[test]
    fn symbol_cse_invalidates_every_written_home() {
        for writer in [
            "xchgq %r9, %r8",
            "xchgq %r8, %r9",
            "xchgl %r9d, %r8d",
            "xchgw %r9w, %r8w",
            "xchgb %r9b, %r8b",
            "mulxq %r8, %r9, %r10",
            "popq %r9",
            "incq %r9",
            "movb $1, %r9b",
            "unknown_effect %r8",
        ] {
            let asm = format!(
                "f:\n.cfi_startproc\n    leaq bins(%rip), %r9\n    {writer}\n    leaq bins(%rip), %r10\n    ret\n.cfi_endproc\n"
            );
            let mut store = super::LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| super::classify_line(store.get(i)))
                .collect();
            assert!(
                !super::cse_symbol_lea(&mut store, &mut infos),
                "stale home after {writer}"
            );
            assert!(!infos[4].is_nop());
            assert_eq!(store.get(4).trim(), "leaq bins(%rip), %r10");
        }
    }

    #[test]
    fn symbol_home_dies_on_implicit_string_comparison_outputs() {
        for mnemonic in ["cmpsb", "cmpsw", "cmpsl", "cmpsq"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    leaq bins(%rip), %rdi\n    {mnemonic}\n    leaq bins(%rip), %r9\n    ret\n.cfi_endproc\n"
            );
            let mut store = super::LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| super::classify_line(store.get(i)))
                .collect();
            assert!(
                !super::cse_symbol_lea(&mut store, &mut infos),
                "stale string home: {mnemonic}"
            );
        }
    }

    #[test]
    fn stable_symbol_materializations_still_cse() {
        for expr in ["bins", "bins+8", "bins-8", ".LC0"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    leaq {expr}(%rip), %r9\n    testq %rdi, %rdi\n    leaq {expr}(%rip), %r10\n    ret\n.cfi_endproc\n"
            );
            let mut store = super::LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| super::classify_line(store.get(i)))
                .collect();
            assert!(
                super::cse_symbol_lea(&mut store, &mut infos),
                "missed {expr}"
            );
            assert_eq!(store.get(4).trim(), "movq %r9, %r10");
        }
        for expr in [
            ".", ".+8", ".-8", "8", "-8", "0x10", "-0x10", "1f", "1b", ".+0x10",
        ] {
            assert!(!super::is_stable_symbol_expr(expr), "accepted {expr}");
        }
    }

    #[test]
    fn rip_lea_home_dies_on_write() {
        // `addq $5, %r9` clobbers the home with a USED value (stored to
        // memory), so the second lea is NOT a redundant redef — two distinct
        // materializations must remain.
        let asm = concat!(
            "f:\n.cfi_startproc\n",
            "    leaq bins(%rip), %r9\n",
            "    addq $5, %r9\n",
            "    addq %r9, %r8\n",
            "    leaq bins(%rip), %r9\n",
            "    movq %r9, (%rdi)\n",
            "    movq %r8, (%rsi)\n",
            "    ret\n.cfi_endproc\n",
        );
        let out = crate::backend::x86::codegen::peephole::peephole_optimize(asm.to_string());
        assert_eq!(
            out.matches("leaq bins(%rip)").count(),
            2,
            "the second lea is NOT redundant after the clobber:\n{out}"
        );
    }

    use super::super::super::peephole_optimize;
    use super::{SibAddr, compose_sib, operand_start, parse_disp, splice_lea_into_mem_operand};

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    fn run_chain_rename(body: &str) -> (Vec<String>, bool) {
        let asm = format!(".text\nf:\n.cfi_startproc\n{body}.cfi_endproc\n");
        let mut store = super::LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        let changed = super::rename_chain_into_copy(&mut store, &mut infos);
        let out = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| !l.starts_with('.') && !l.ends_with(':'))
            .collect();
        (out, changed)
    }

    /// The chain's producer copies the copy's own destination: renaming the
    /// scratch into `%r13` must not leave the no-op `movq %r13, %r13`
    /// (hot_web_loop's `d = rotl(d, 7) ^ c` emitted it in every iteration).
    #[test]
    fn chain_rename_drops_the_64_bit_self_move_it_creates() {
        let (out, changed) = run_chain_rename(concat!(
            "    movq %r13, %rdi\n",
            "    rolq $7, %rdi\n",
            "    xorq %rbp, %rdi\n",
            "    movq %rdi, %r13\n",
            "    movq %r13, %rax\n",
            "    ret\n",
        ));
        assert!(changed);
        assert_eq!(
            out,
            ["rolq $7, %r13", "xorq %rbp, %r13", "movq %r13, %rax", "ret"]
        );
    }

    /// The 32-bit form is not a no-op: `movl %r13d, %r13d` clears bits 32..63
    /// exactly as the original `movl %r13d, %edi` did, so it must survive.
    #[test]
    fn chain_rename_keeps_the_zero_extending_32_bit_self_move() {
        let (out, changed) = run_chain_rename(concat!(
            "    movl %r13d, %edi\n",
            "    roll $7, %edi\n",
            "    movl %edi, %r13d\n",
            "    movq %r13, %rax\n",
            "    ret\n",
        ));
        assert!(changed);
        assert_eq!(
            out,
            [
                "movl %r13d, %r13d",
                "roll $7, %r13d",
                "movq %r13, %rax",
                "ret"
            ]
        );
    }

    #[test]
    fn q_self_move_predicate() {
        assert!(super::is_q_self_move("movq %r13, %r13"));
        assert!(!super::is_q_self_move("movq %r13, %r12"));
        assert!(!super::is_q_self_move("movl %r13d, %r13d"));
        assert!(!super::is_q_self_move("movq (%r13), %r13"));
        assert!(!super::is_q_self_move("movq $1, $1"));
    }

    fn run_lea_base_fold(asm: &str) -> (String, bool) {
        let mut store = super::LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        let changed = super::fold_copy_into_lea_base(&mut store, &mut infos);
        let out = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\\n");
        (out, changed)
    }

    /// A `movl` load whose value is relayed once into the register the loop
    /// actually uses may retarget the load into that register and drop the
    /// relay — including when the loaded-from register is `%rbp` used as a
    /// plain callee-saved GPR (no frame-pointer setup, no rbp slot traffic).
    /// This is the `rot` entry shape.
    #[test]
    fn load_relay_into_gpr_rbp_home_retargets() {
        let out = run(concat!(
            "rot:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    movl 28(%rdi), %ebp\n",
            "    movq %rbp, %rdx\n",
            "    movl %edx, (%rsi)\n",
            "    popq %rbp\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movl 28(%rdi), %edx"),
            "load must retarget into the relay home:\n{out}"
        );
        assert!(
            !out.contains("movq %rbp, %rdx"),
            "the relay must be gone:\n{out}"
        );
    }

    /// The same shape under a FRAME-POINTER build must keep the LOAD homed in
    /// the frame register: rbp carries the frame, so the load may not be
    /// retargeted into the relay home. The relay itself is fair game for the
    /// always-on redundant-zero-extension elimination — `movl 28(%rdi),%ebp`
    /// zero-extends, so the follow-up `movq %rbp,%rdx` may respell to
    /// `movl %ebp,%edx` (identical 64 bits) — and a single-use store may then
    /// relay the copy source through (`movl %ebp,(%rsi)`). The narrowed copy
    /// survives because %rdx is conservatively live at `ret` (SysV 128-bit
    /// integer returns). What this test actually pins is the soundness
    /// invariant: the frame register is never the retarget victim.
    #[test]
    fn load_relay_under_frame_pointer_load_stays_in_rbp() {
        let out = run(concat!(
            "rot:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    movl 28(%rdi), %ebp\n",
            "    movq %rbp, %rdx\n",
            "    movl %edx, (%rsi)\n",
            "    movq %rbp, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movl 28(%rdi), %ebp"),
            "frame-homed load must stay in rbp:\n{out}"
        );
        assert!(
            !out.contains("28(%rdi), %edx"),
            "the load must not retarget into the relay home under a frame pointer:\n{out}"
        );
        // The store happens, via the relay or through the relayed copy source.
        assert!(
            (out.contains("movq %rbp, %rdx") && out.contains("movl %edx, (%rsi)"))
                || out.contains("movl %ebp, (%rsi)")
                || out.contains("movl %edx, (%rsi)"),
            "the store must read the loaded value:\n{out}"
        );
        // The frame itself is intact.
        for frame_line in [
            "pushq %rbp",
            "movq %rsp, %rbp",
            "movq %rbp, %rsp",
            "popq %rbp",
        ] {
            assert!(
                out.contains(frame_line),
                "frame line `{frame_line}` lost:\n{out}"
            );
        }
    }

    #[test]
    fn relay_copy_is_folded_into_alu_source() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rsi,%rbx), %eax\n",
            "    movl %eax, %r10d\n",
            "    addl %r10d, %r8d\n",
            // The store keeps %r8d — and therefore the folded add — live:
            // `eliminate_dead_flag_writes` retires a consumer whose result
            // nothing observes, and this test is about the FOLD.
            "    movl %r8d, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addl %eax, %r8d"), "{out}");
        assert!(!out.contains("%r10d"), "{out}");
    }

    #[test]
    fn both_readers_of_a_relayed_value_see_the_same_register() {
        // The relay has TWO consumers, so it may not be retired by rewriting
        // only one of them -- that would leave the second reading a register
        // nobody wrote.
        //
        // Asserting the exact text `movl %eax, %r10d` would be wrong here:
        // `copy_fold` retires the relay by rewriting BOTH consumers to `%eax`
        // (which holds the same value and outlives them), giving three
        // instructions instead of four. What must hold is the invariant, not
        // the spelling: both adds read one and the same register, and it is a
        // register that actually holds the relayed value.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r10d\n",
            "    addl %r10d, %r8d\n",
            "    addl %r10d, %r9d\n",
            // Both results must be observable for the relay invariant to be
            // testable at all: a consumer whose result nothing reads is now
            // (correctly) deleted.
            "    movl %r8d, (%rdx)\n",
            "    movl %r9d, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        let src_of = |dst: &str| -> String {
            out.lines()
                .find(|l| l.trim().starts_with("addl") && l.trim().ends_with(dst))
                .unwrap_or_else(|| panic!("missing `addl ..., {dst}`:\n{out}"))
                .trim()
                .trim_start_matches("addl ")
                .split(',')
                .next()
                .unwrap()
                .trim()
                .to_string()
        };
        let a = src_of("%r8d");
        let b = src_of("%r9d");
        assert_eq!(a, b, "both consumers must read the same register:\n{out}");
        assert!(
            a == "%r10d" || a == "%eax",
            "consumers must read the relayed value, got {a}:\n{out}"
        );
        // If the relay survived, it must still be written before both uses.
        if a == "%r10d" {
            assert!(out.contains("movl %eax, %r10d"), "{out}");
        }
    }

    #[test]
    fn copy_to_caller_saved_scratch_is_dead_across_a_plain_call() {
        // SysV: %r10 is caller-saved scratch. An UNMARKED `call bar` clobbers
        // it, so the value staged by `movl %eax, %r10d` can never reach the
        // post-call read -- the copy is dead under the ABI and must be
        // eliminated. (Chain calls -- the only callees that read a staged
        // %r10 -- carry `# LCCC_CHAIN_CALL`, see
        // relay_is_kept_when_a_call_reads_the_static_chain.) The consumer
        // itself must survive untouched: it reads whatever `bar` left in
        // %r10d with or without the copy.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r10d\n",
            "    call bar\n",
            "    addl %r10d, %r8d\n",
            // Keep the post-call consumer observable (its result is what the
            // test contrasts with the dead staging copy).
            "    movl %r8d, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %eax, %r10d"), "{out}");
        assert!(out.contains("addl %r10d, %r8d"), "{out}");
    }

    #[test]
    fn relay_to_callee_saved_reg_survives_a_call() {
        // The load-bearing inverse of the dead-scratch case: %ebx survives
        // the call, so the staged value DOES reach the post-call consumer
        // and the relay may be neither folded away (the source %eax dies
        // at the call) nor dropped (the destination is read after it).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %ebx\n",
            "    call bar\n",
            "    addl %ebx, %r8d\n",
            // The consumer's result must escape, or the dead-result rule
            // removes the very instruction this test needs to observe.
            "    movl %r8d, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %eax, %ebx"), "{out}");
        assert!(out.contains("addl %ebx, %r8d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_copy_and_renames_later_use() {
        // The sqlite-varint exit shape: `movl` copy into an `andl` consumer
        // with a second use of the copy dest further down.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    orl %r10d, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $2080895, %r8d"), "{out}");
        assert!(out.contains("orl %r8d, %edi"), "{out}");
        assert!(!out.contains("%r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_the_call_result_relay_into_the_return_register() {
        // `return use(a) + g;`: GCC 16.2 `call use; addl g(%rip), %eax`.
        // The dword consumer zero-extends, so the copy back is the identity.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    addl g(%rip), %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addl g(%rip), %eax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    #[test]
    fn rmw_coalesce_keeps_the_zero_extension_after_a_qword_write() {
        // The last write of the coalesced value is a qword shift: its upper
        // half may be nonzero, so the dword copy back still truncates and
        // must survive as `movl %eax, %eax`.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    subl %edi, %r11d\n",
            "    shlq $3, %r11\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // (`subl`: no `lea` form, so an earlier pass cannot pre-empt this one.)
        assert!(out.contains("subl %edi, %eax"), "{out}");
        assert!(out.contains("shlq $3, %rax"), "{out}");
        assert!(out.contains("movl %eax, %eax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    #[test]
    fn rmw_coalesce_copy_back_rolls_back_when_the_copy_stays_live() {
        // %r11 is read after the copy back: the deadness proof fails and
        // the whole rewrite (copy, consumer, dropped copy back) is undone.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    subl %edi, %r11d\n",
            "    movl %r11d, %eax\n",
            "    imull %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Whatever else runs, the difference must still be squared: it is
        // read twice, from one register or from two holding it.
        assert!(
            out.contains("subl %edi, %r11d") || out.contains("subl %edi, %eax"),
            "{out}"
        );
        assert!(
            out.contains("imull %r11d, %eax") || out.contains("imull %eax, %eax"),
            "{out}"
        );
    }

    /// `movl %S, %D` relays into a 32-bit LEA with no upper-half proof: the
    /// `leal` result depends only on the low 32 bits of its address
    /// registers. This is `use(a) * 3` (GCC: `leal (%rax,%rax,2), %eax`).
    #[test]
    fn move_relay_into_leal_needs_no_upper_proof() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    leal (%r11, %r11, 2), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal (%rax, %rax, 2), %eax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    /// Control: a 64-bit `leaq` reads the upper half the `movl` copy zeroed
    /// and `%rax` (a call result) does not provably have zero there, so the
    /// copy must stay.
    #[test]
    fn move_relay_into_leaq_keeps_the_zero_extending_copy() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    leaq (%r11, %r11, 2), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("(%rax, %rax, 2)"), "{out}");
    }

    /// The 64-bit twin: a `movq` copy relays into a self-addressed `leaq`
    /// whose destination is the copy (`leaq (%r11,%r11,2), %r11`): the LEA
    /// reads the copy only through its address, which the rename retargets.
    #[test]
    fn move_relay_into_self_addressed_leaq() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movq %rax, %r11\n",
            "    leaq (%r11, %r11, 2), %r11\n",
            "    leaq 1(%r11), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movq %rax, %r11"), "{out}");
        assert!(out.contains("(%rax, %rax, 2)"), "{out}");
    }

    /// `usel(a) * 3 + 1` at 64 bits: the multiply LEA composes into the
    /// displaced `leaq 1(%T)` (GCC: `leaq 1(%rax,%rax,2), %rax`).
    #[test]
    fn lea_composes_into_displaced_lea_use() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    leaq (%rax, %rax, 2), %r11\n",
            "    leaq 1(%r11), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rax, %rax, 2), %rax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    /// Displacements add (including to zero), for loads too.
    #[test]
    fn lea_composes_into_displaced_load_summing_displacements() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq -8(%rdi, %rsi, 4), %r11\n",
            "    movl 12(%r11), %eax\n",
            "    movl 8(%rdi), %ecx\n",
            "    leaq 8(%rcx, %rdx, 8), %r10\n",
            "    addl -8(%r10), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl 4(%rdi, %rsi, 4), %eax"), "{out}");
        assert!(out.contains("(%rcx, %rdx, 8), %eax"), "{out}");
        assert!(!out.contains("%r11") && !out.contains("%r10"), "{out}");
    }

    /// The consumer may overwrite an address register (the address is
    /// read first) — but never through %rsp-adjusting stack instructions.
    #[test]
    fn lea_relay_consumer_may_write_an_address_register_except_stack_ops() {
        let ok = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq (%rax, %rbx, 1), %r11\n",
            "    movq (%r11), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            ok.contains("movq (%rax, %rbx, 1), %rax") || ok.contains("movq (%rax,%rbx), %rax"),
            "{ok}"
        );
        let pop = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 8(%rsp), %r11\n",
            "    popq (%r11)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(pop.contains("popq (%r11)"), "{pop}");
    }

    /// Fail closed: a RIP-relative producer (relocation displacement), a
    /// live `%T`, and an out-of-range sum all keep the LEA.
    #[test]
    fn displaced_relay_composition_refusals() {
        let rip = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq tbl(%rip), %r11\n",
            "    movl 8(%r11), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!rip.contains("8tbl") && !rip.contains("tbl8"), "{rip}");
        let live = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq (%rdi, %rsi, 4), %r11\n",
            "    movl 8(%r11), %eax\n",
            "    addl (%r11), %eax\n",
            "    movq %r11, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(live.contains("movq %r11, (%rdx)"), "{live}");
        let big = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 2147483647(%rdi, %rsi, 1), %r11\n",
            "    movl 1(%r11), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!big.contains("2147483648("), "{big}");
    }

    /// End to end, the `use(a) * 3 + 1` shape: the relay folds into the
    /// `leal`, the `addl $1` folds into its displacement, and the copy back
    /// retargets — GCC's single `leal 1(%rax,%rax,2), %eax`.
    #[test]
    fn mul3_plus1_relay_collapses_to_one_leal() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    call bar\n",
            "    movl %eax, %r11d\n",
            "    leal (%r11, %r11, 2), %r11d\n",
            "    addl $1, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal 1(%rax, %rax, 2), %eax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
        assert!(!out.contains("addl"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_64bit_copy() {
        // `subq` (non-commutative) defeats `load_alu_fuse`'s copy+commute,
        // and the later full redefinition of %r12 defeats whole-function
        // `copy_coalesce` (rule 2) without making the source live.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r12, %r11\n",
            "    subq %r9, %r11\n",
            "    movq %r11, (%rsi)\n",
            "    call bar\n",
            "    movq $5, %r12\n",
            "    addq %r12, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("subq %r9, %r12"), "{out}");
        assert!(out.contains("movq %r12, (%rsi)"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_32bit_consumer_under_64bit_copy() {
        // A 32-bit consumer under a 64-bit copy reads only the established
        // low half: sound, and the rewrite zero-extends identically. (The
        // wide mask defeats `copy_mask_movz`; the later %r12 redefinition
        // defeats whole-function `copy_coalesce`. The consumer lands in the
        // CALLEE-SAVED %ebx and is used after the call: a caller-saved
        // destination would be dead at the call — non-variadic calls write
        // %rax and read no accumulator — and would be eliminated, not
        // retargeted; keeping it live pins the retarget itself.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r12, %r11\n",
            "    andl $2080895, %r11d\n",
            "    movl %r11d, %ebx\n",
            "    call bar\n",
            "    addq %rbx, %rax\n",
            "    movq $5, %r12\n",
            "    addq %r12, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $2080895, %r12d"), "{out}");
        assert!(out.contains("movl %r12d, %ebx"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_64bit_consumer_under_32bit_copy() {
        // The 64-bit shift reads the source's unknown upper half: unsound to
        // retarget, so the copy must stay (whatever other passes do around
        // it).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r9d\n",
            "    shlq $32, %r9\n",
            "    movq %r9, (%rsi)\n",
            "    movl $0, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %eax, %r9d"), "{out}");
        assert!(out.contains("shlq $32, %r9"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq31() {
        // Count 31 is value-unsound: unfolded bit 63 is 0 (shifted out of
        // the zero-extended low half), folded it is the source's bit 32.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r9d\n",
            "    shlq $31, %r9\n",
            "    orq %r9, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r9d"), "{out}");
        assert!(out.contains("shlq $31, %r9"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_loop_carried_source() {
        // Torture builtin-bitops-1: the copy sits in a loop with a
        // loop-invariant source (%rdi mentioned only here). Coalescing
        // would retarget the consumer into %rdi, so iteration N+1 reads
        // iteration N's result instead of the invariant.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB2:\n",
            "    movq %rdi, %r9\n",
            "    andq %r8, %r9\n",
            "    testq %r9, %r9\n",
            "    cmovneq %r11, %rdx\n",
            "    addq $1, %rsi\n",
            "    cmpq $64, %rsi\n",
            "    jl .LBB2\n",
            "    movq %rdx, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rdi, %r9"), "{out}");
        assert!(out.contains("andq %r8, %r9"), "{out}");
        assert!(!out.contains("andq %r8, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq64_count_masking() {
        // Silicon shifts by `imm & 63`: `$64` shifts by ZERO (a no-op), so
        // unfolded `%r9` is zero-extended while folded `%r8` is not. A naive
        // `imm >= 32` check miscompiles this; the masked check refuses even
        // with a clobber (`orq`) present.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r9d\n",
            "    shlq $64, %r9\n",
            "    orq %r9, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r9d"), "{out}");
        assert!(out.contains("shlq $64, %r9"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_adcl() {
        // `adcl` reads the divergent CF before any clobber: veto.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    adcl $0, %ecx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_push_barrier() {
        // Push/pop are barriers: unknown code past them could read CF, so
        // the fold needs a clobber BEFORE the barrier. None here: veto.
        // The pair is kept alive by the `orq %r12` below (so no deadness
        // proof can retire it) and the audit runs in the width gate, before
        // either deadness proof, so the refusal isolates the veto.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            // %r12, not %r10: an unmarked `call` clobbers caller-saved
            // registers, so a pair staged in %r10 is legitimately dead under
            // the ABI (`eliminate_dead_flag_writes` deletes it) and the veto
            // under test would be unobservable.  %r12 survives the call.
            "    movl %r8d, %r12d\n",
            "    shlq $32, %r12\n",
            "    pushq %rbx\n",
            "    call qux\n",
            "    popq %rbx\n",
            "    movl %ebx, %eax\n",
            "    orq %r12, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("pushq %rbx"), "{out}");
        assert!(out.contains("movl %r8d, %r12d"), "{out}");
        assert!(out.contains("shlq $32, %r12"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_seta() {
        // SETA is a COMBINED CF+ZF predicate; it must veto just like SETB.
        // The `set` prefix-skip used to classify it as a non-CF read.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    seta %cl\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_cmova() {
        // CMOVA reads CF (and ZF); cmovcc lines are NOT CFG barriers, so
        // the `cmov` prefix-skip was an actual carry audit hole.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    cmova %r8d, %edx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_setbe() {
        // SETBE (CF OR ZF) is the second combined-predicate direction.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    setbe %cl\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_when_andnl_kills_carry() {
        // ANDN clears CF/OF (BMI1), so a later SETA observes only flags
        // produced by ANDN — the divergent shift carry is dead and the
        // pair folds. Regression: andnl/andnq carried no clobber credit
        // (only the suffix-less token was listed).
        // The push/pop spill makes the snippet ABI-valid (Guard-1(b)
        // refuses save-less callee-saved folds); dataflow proves %ebx
        // dead after the copy via the pop kill.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    movl %ebx, %r11d\n",
            "    shlq $32, %r11\n",
            "    andnl %edx, %edi, %edi\n",
            "    seta %cl\n",
            "    movq %r11, %rax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %ebx"), "{out}");
        assert!(out.contains("shlq $32, %rbx"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_when_pdep_preserves_carry() {
        // PDEP/PEXT leave CF untouched (silicon-verified), so a SETA after
        // one still observes the shift's divergent carry: refuse.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    pdepl %edx, %edi, %edi\n",
            "    seta %cl\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_when_blsil_overwrites_carry() {
        // BLSI writes CF=1 regardless of inputs (silicon-verified), so
        // the divergent shift carry is dead once BLSI executes.
        // The push/pop spill makes the snippet ABI-valid (Guard-1(b)
        // refuses save-less callee-saved folds); dataflow proves %ebx
        // dead after the copy via the pop kill.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    movl %ebx, %r11d\n",
            "    shlq $32, %r11\n",
            "    blsil %edi, %edi\n",
            "    seta %cl\n",
            "    movq %r11, %rax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %ebx"), "{out}");
        assert!(out.contains("shlq $32, %rbx"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_when_ptest_kills_carry() {
        // PTEST/VPTEST write CF from the masked AND; clobber credit.
        // The push/pop spill makes the snippet ABI-valid (Guard-1(b)
        // refuses save-less callee-saved folds); dataflow proves %ebx
        // dead after the copy via the pop kill.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    movl %ebx, %r11d\n",
            "    shlq $32, %r11\n",
            "    ptest %xmm0, %xmm1\n",
            "    seta %cl\n",
            "    movq %r11, %rax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %ebx"), "{out}");
        assert!(out.contains("shlq $32, %rbx"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_setl_signed_compare() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // `setl` is the OF reader lccc emits for every signed compare
        // (backend/x86/codegen/comparison.rs, i128_ops.rs). It is not a CFG
        // barrier, and the CF audit's `set` prefix skip stepped over it: with
        // a CF+OF clobber (`addl`) waiting below, a CF-only audit ACCEPTED the
        // fold and fed the divergent OF (the source's bit 63) into the signed
        // relation. Refusing is what makes the prefix skip safe.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    setl %cl\n",
            "    addl %eax, %ebx\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
        assert!(!out.contains("shlq $32, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_when_the_of_reader_is_replaced_by_a_writer() {
        // Control for the test above, identical shape with `addl` where `setl`
        // was: ADD writes OF (silicon-verified), so both divergent flags are
        // dead before anything observes them and the fold stands. The refusal
        // above is therefore the OF READ, not the mnemonic family or the shape.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    addl %eax, %ebx\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("shlq $32, %r8"), "{out}");
        assert!(out.contains("movq %r8, %rax"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_adoxq_of_consumer() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // ADOX consumes OF as its carry-in, so the divergence reaches a VALUE,
        // not just a flag: measured natively, `shlq $32` unfolded vs folded
        // with src `0x8000_0000_8000_0000` gives OF 0 vs 1 and a following
        // `adoxq` returns 0 vs 1. `adox` sits in CF_SKIP_EXACT — correctly, it
        // neither reads nor writes CF — so a CF-only audit skipped it and
        // accepted at the `addl` below. The suffixed spelling is what GAS
        // emits and what the exact-match table must name.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    adoxq %r9, %rdi\n",
            "    addl %eax, %ebx\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("adoxq %r9, %rdi"), "{out}");
        assert!(!out.contains("shlq $32, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_when_clc_kills_cf_but_not_of() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // Partial credit must not end the audit. CLC writes CF — settling the
        // CF question — and preserves OF (silicon-verified, as do `stc`,
        // `sahf` and the `bt` family), so the divergent OF is still live at
        // the `seto` below. A single-flag scan returned true at the CLC and
        // folded; the dual-flag scan keeps going and vetoes at the reader.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    clc\n",
            "    seto %cl\n",
            "    addl %eax, %ebx\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("seto %cl"), "{out}");
        assert!(!out.contains("shlq $32, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_when_clc_then_addl_kill_both_flags() {
        // The shape the partial-credit continuation exists for: CF dies at
        // `clc`, OF dies at `addl`, so both divergent flags are dead before
        // the `seto` observes them and the fold stands. Without the
        // continuation the scan would default-deny at the CLC (it is neither a
        // skip-list entry nor a prefix match), losing this fold.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    clc\n",
            "    addl %eax, %ebx\n",
            "    seto %cl\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("shlq $32, %r8"), "{out}");
        assert!(out.contains("clc"), "{out}");
        assert!(out.contains("seto %cl"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_cond_jump() {
        // A conditional jump ends the knowable region (CF flows to both
        // successors) with no clobber seen: veto.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    jc .Lx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_before_call() {
        // Calls kill flag knowledge (LLVM parity: the callee observes
        // scratch flags), so the divergent CF is dead at the `call` and the
        // pair folds. The push/pop spill makes the snippet ABI-valid: dataflow
        // proves %ebx dead after the copy (the pop kills it, nothing rereads
        // it) even though uniqueness must refuse callee-saved sources (see
        // the next test). An argument-register or static-chain (`%r10`)
        // source would stay live into the call and refuse on the value proof
        // instead.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    movl %ebx, %r11d\n",
            "    shlq $32, %r11\n",
            "    movq %r11, %rdi\n",
            "    call bar\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %ebx"), "{out}");
        assert!(out.contains("shlq $32, %rbx"), "{out}");
        assert!(out.contains("movq %rbx, %rdi"), "{out}");
        assert!(out.contains("call bar"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_callee_saved_source_without_spill() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // %ebx is owed its entry value at `ret` (RET_LIVE must-restore), an
        // obligation no text scan can see: whole-function uniqueness must not
        // justify the new write even though %ebx is mentioned only at the
        // copy. (The pre-fix test above folded this exact shape.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %r11d\n",
            "    shlq $32, %r11\n",
            "    movq %r11, %rdi\n",
            "    call bar\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %ebx, %r11d"), "{out}");
        assert!(out.contains("shlq $32, %r11"), "{out}");
        assert!(!out.contains("shlq $32, %rbx"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_into_call_arg_reg() {
        // The sound backstop of the test above: with `%r9` (argument
        // register 6) as the destination, folding would leak the OLD `%r9`
        // into the callee as an argument (unfolded the copy kills it). The
        // destination-dead rollback refuses. (The source is callee-saved,
        // hence dead into the call, so the source proof holds and the
        // refusal isolates the destination rollback.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %r9d\n",
            "    shlq $32, %r9\n",
            "    movq %r9, %rdi\n",
            "    call bar\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %ebx, %r9d"), "{out}");
        assert!(out.contains("shlq $32, %r9"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_before_ret() {
        // Flags at `ret` are undefined (LLVM parity), so the divergent CF
        // is dead there and the pair folds.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("shlq $32, %r8"), "{out}");
        assert!(out.contains("movq %r8, %rax"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_after_inc_only() {
        // The classic pitfall: `incl` preserves CF (NOT a clobber), so the
        // `adcl` still reads the divergent carry: veto.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    incl %eax\n",
            "    adcl $0, %ecx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shrq32() {
        // The documented asymmetry: right shifts READ the zeroed upper
        // half, so no cross-family fold exists for them.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shrq $32, %r10\n",
            "    orq %r10, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shrq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_clobbered_by_testl() {
        // The clobber need not be an RMW: `testl` kills CF and its
        // destination-read renames like any other.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    testl %r10d, %r10d\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("%r10"), "{out}");
        assert!(out.contains("testl %r8d, %r8d"), "{out}");
        assert!(out.contains("movl %r8d, %eax"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_with_rdx_source_and_marker() {
        // Marker cooperation: `%rdx`-sourced cross-family folds need the
        // return-type fact (the detector alone keeps `%rdx` live).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    movl %edx, %r10d\n",
            "    shlq $32, %r10\n",
            "    orq %r10, %rax\n",
            "    movq %rax, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %edx"), "{out}");
        assert!(out.contains("shlq $32, %rdx"), "{out}");
        assert!(out.contains("orq %rdx, %rax"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq_cl_count() {
        // A `%cl` count cannot be proven 32: refuse.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq %cl, %r10\n",
            "    orq %r10, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq %cl, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_unknown_mnemonic() {
        // Default-deny: an unclassified mnemonic (here AVX) before the
        // clobber vetoes — it may read CF.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    vpbroadcastd %xmm0, %ymm1\n",
            "    orq %r10, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_before_uppercase_adcq() {
        // Case-insensitivity where it matters: an odd-case CF reader still
        // vetoes (the consumer matcher stays case-sensitive — missed folds
        // are safe, missed readers are not).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    ADCQ $0, %RCX\n",
            "    orq %r10, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_shlq32_after_shlx_only() {
        // The BMI2 trap: `shlx` does NOT touch flags (NOT a clobber), so
        // the `adcl` still reads the divergent carry: veto.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    shlx %eax, %ebx, %ecx\n",
            "    adcl $0, %ecx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shlq32_skipped_inc_then_clobber() {
        // Skip-then-clobber: the `incl` (CF-preserving) is skipped, the
        // `orq` clobbers, and the pair folds.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    incl %eax\n",
            "    orq %r10, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            // `movl %eax, (%rdx)` instead of `movl $5, %eax`: it observes the
            // incremented %eax, so the CF-preserving `incl` cannot be retired
            // as a dead write before the audit this test exercises.
            "    movl %eax, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("incl %eax"), "{out}");
        assert!(out.contains("orq %r8, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_orq64_proven_zero_upper() {
        // P3 flagship: BOTH halves of the exit-compose fold in one run.
        // First `movl %r8d, %r9d; shlq $32, %r9` (P2.5), then `movl %r10d,
        // %edi; orq %r8, %rdi` (P3: `%r10`'s upper half is proven zero by
        // the `orl` above — every dword write zero-extends).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    shll $7, %r11d\n",
            "    orl %r11d, %r10d\n",
            "    shrl $18, %r8d\n",
            "    movl %r8d, %r9d\n",
            "    shlq $32, %r9\n",
            "    movl %r10d, %edi\n",
            "    orq %r9, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $5, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r8"), "{out}");
        assert!(out.contains("orq %r8, %r10"), "{out}");
        assert!(out.contains("movq %r10, (%rsi)"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_entry_param() {
        // The SysV trap: an entry parameter's upper half is garbage (the
        // caller need not zero it), so "no definer found" refuses.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %edi, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %edi, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_sign_extended_definer() {
        // A 64-bit sign-extending definer proves nothing about zero: the
        // upper half is sign bits, possibly nonzero. (The 64-bit store
        // keeps an earlier pass from narrowing the `movslq` to `movl`.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %eax, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rdi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movslq %eax, %r8"), "{out}");
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_partial_write_definer() {
        // A partial (byte) write preserves the upper half: not a prover.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movb $1, %r8b\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_clobbered_between() {
        // Nearest-definer semantics: the 32-bit `orl` below is shadowed by
        // the 64-bit `orq` clobber, which proves nothing. (An RMW clobber,
        // not a copy: copy propagation would forward a `movq` clobber into
        // the copy and dissolve the shape before this pass runs.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movl %r10d, %edi\n",
            "    orq %rbx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("orq %rax, %r10"), "{out}");
        assert!(out.contains("movl %r10d, %edi"), "{out}");
        assert!(out.contains("orq %rbx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_orq64_xor_self_definer() {
        // The zeroing idiom proves the upper half (`xorq %r8, %r8` zeroes
        // all 64 bits).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorq %r8, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("xorq %r8, %r8"), "{out}");
        assert!(out.contains("orq %rax, %r8"), "{out}");
        assert!(out.contains("movq %r8, (%rsi)"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_orq64_small_imm_definer() {
        // A small nonnegative `movq $imm` (sign-extended, hence still
        // small) proves the upper half.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq $5, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("orq %rax, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_negative_imm_definer() {
        // The sign-extension trap: `movq $-1` fills the upper half with
        // ones (GAS sign-extends imm32), so it must refuse.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq $-1, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_big_hex_imm_definer() {
        // Same trap in hex clothing: `$0xFFFFFFFF` is -1 once GAS
        // sign-extends it into the qword.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq $0xFFFFFFFF, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_addq64_proven() {
        // P3 is not or-specific: any matched 64-bit RMW folds under the
        // proof (exact value equality covers every opcode).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    movl %r10d, %edi\n",
            "    addq %rax, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("addq %rax, %r10"), "{out}");
        assert!(out.contains("movq %r10, (%rsi)"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_across_call() {
        // The scan stops at calls (uniformly — even where a finer proof
        // might survive): no visible definer, no fold. Correct here in any
        // case: the call clobbers `%r10`.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    call qux\n",
            "    movl %r10d, %edi\n",
            "    orq %rax, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edi"), "{out}");
        assert!(out.contains("orq %rax, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_uppercase_definer() {
        // Odd-case text refuses: the reference scan is case-sensitive, so
        // an unseen same-case write could shadow this definer.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    ORL %R11D, %R10D\n",
            "    movl %r10d, %edi\n",
            "    orq %rax, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edi"), "{out}");
        assert!(out.contains("orq %rax, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_shrq64_proven() {
        // Right shifts fold WITH the proof (contrast the unproven `shrq`
        // refusal): `%S64 ≡ %D64` makes every consumer opcode safe.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    movl %r10d, %r9d\n",
            "    shrq $8, %r9\n",
            "    movq %r9, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("shrq $8, %r10"), "{out}");
        assert!(out.contains("movq %r10, (%rsi)"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_implicit_between() {
        // Implicit-register lines stop the scan uniformly (even when the
        // implicit traffic provably targets other families).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    mulq %rcx\n",
            "    movl %r10d, %edi\n",
            "    orq %rax, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edi"), "{out}");
        assert!(out.contains("orq %rax, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_orq64_read_between() {
        // Reads between the definer and the copy are fine (they change
        // nothing); only writes re-anchor the proof.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    cmpl %r10d, %eax\n",
            "    movl %r10d, %edi\n",
            "    orq %rax, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
        assert!(out.contains("movq %r10, (%rsi)"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_imulq64_consumer() {
        // `imulq` is absent from the consumer table (like `salq`): the
        // candidate never matches, pinning the gap for the table-completion
        // batch rather than silently folding or refusing downstream.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    movl %r10d, %r9d\n",
            "    imulq %rax, %r9\n",
            "    movq %r9, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %r9d"), "{out}");
        assert!(out.contains("imulq %rax, %r9"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_movl_orq64_movabs_small() {
        // A small `movabsq $imm64` (low 32 bits only) proves the upper half.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movabsq $5, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r8d"), "{out}");
        assert!(out.contains("orq %rax, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_movl_orq64_movabs_big() {
        // A `movabsq` with bits above 32 set proves nothing.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movabsq $0x100000000, %r8\n",
            "    movl %r8d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("orq %rax, %r10"), "{out}");
    }

    #[test]
    fn move_relay_folds_movl_orq64_read_widening_proven() {
        // P4 flagship: the varint survivors (copy + pure-read consumer).
        // `movl %r10d, %edx; orq %rdx, %r8` relays to `orq %r10, %r8`: the
        // 64-bit read needs the P3 zero-upper proof (the `orl` above), and
        // `%rdx` needs the return-type fact (the detector alone keeps it
        // live — the marker models emitter output).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    orl %r11d, %r10d\n",
            "    shrl $18, %r8d\n",
            "    shlq $32, %r8\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    movl $6, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("orq %r10, %r8"), "{out}");
    }

    #[test]
    fn move_relay_refuses_movl_orq64_read_widening_unproven() {
        // A 64-bit read under a 32-bit copy of an entry parameter (garbage
        // upper half): the widening proof fails, the copy stays.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %edi, %edx\n",
            "    orq %rdx, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %edi, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %r8"), "{out}");
    }

    #[test]
    fn move_relay_folds_movq_cmpl32_narrowing_free() {
        // Narrowing needs NO proof: `movq` preserves the low 32 bits, so a
        // 32-bit read relays free. (`setl` consumes the flags so no other
        // pass deletes the `cmpl` as a dead flag-write.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rsi, %r10\n",
            "    cmpl %r10d, %eax\n",
            "    setl %cl\n",
            "    movzbl %cl, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movq %rsi, %r10"), "{out}");
        assert!(out.contains("cmpl %esi, %eax"), "{out}");
    }

    #[test]
    fn move_relay_folds_movl_store_widening_proven() {
        // The store arm widens too: `movl %r10d, %edx; movq %rdx, MEM` →
        // `movq %r10, MEM` under the zero-upper proof (+ the marker for the
        // `%rdx` destination-dead proof).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    orl %r11d, %r10d\n",
            "    movl %r10d, %edx\n",
            "    movq %rdx, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("movq %r10, (%rsi)"), "{out}");
    }

    #[test]
    fn move_relay_refuses_high_byte_use() {
        // The high-byte trap: `%dh` reads bits 8-15, which no copy
        // establishes — the use must not relay to `%S`'s low byte. (The
        // fragment is invalid x86 — GAS rejects `orl` on byte regs — but
        // the pass must refuse garbage, never misrelay it.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %edx\n",
            "    orl %dh, %ecx\n",
            "    movl %ecx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %eax, %edx"), "{out}");
        assert!(out.contains("orl %dh, %ecx"), "{out}");
    }

    #[test]
    fn move_relay_folds_movl_adcq64_widening_proven() {
        // Carry flows through the same line unchanged, so `adcq` widens
        // under the proof like any other consumer.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    orl %r11d, %r10d\n",
            "    movl %r10d, %edx\n",
            "    adcq %rdx, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("adcq %r10, %r8"), "{out}");
    }

    #[test]
    fn move_relay_refuses_movl_orq64_clobbered_definer() {
        // Nearest-definer semantics on the relay path too: the 64-bit
        // `orq` clobber shadows the proving `orl` below it.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    orl %r11d, %r10d\n",
            "    orq %rax, %r10\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %r8"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_cmovl_definer() {
        // `cmovl` leaves the dest unchanged when the condition is false, so
        // the old (unknown) upper half may survive: not a zero-upper proof.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cmovae %eax, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_cmpxchgl_definer() {
        // `cmpxchg` leaves the dest unchanged on the failed path: the upper
        // half keeps its old (unknown) value. (`lock` strips transparently.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    lock cmpxchgl %ecx, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_bsfl_definer() {
        // `bsf` with a zero source leaves the dest unmodified: not a proof.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    bsfl %ecx, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_rdrand_definer() {
        // `rdrand` leaves the dest unchanged when it fails (CF=0).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    rdrand %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_data16_definer() {
        // Hand-asm size-prefix trickery: `data16 orl` executes a 16-bit op,
        // so the textual dword dest proves nothing about the upper half.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    data16 orl %r11d, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_lock_orl_definer() {
        // `lock` is width-transparent (and `orl` unconditional): a locked
        // dword write still proves zero-upper. Pins no-over-deny. (Relay
        // shape — the marker models the emitter's return-type fact.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    lock orl %r11d, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("orq %r10, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_shlq32_proven_upper_folds_past_adcl() {
        // Arm composition: a 32-step `shlq` with PROVEN zero-upper folds via
        // P3 (exact value equality needs no flag audit), even though an
        // `adcl` below would veto the P2.5 unproven path.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %r10d, %r10d\n",
            "    movl %r10d, %edx\n",
            "    shlq $32, %rdx\n",
            "    adcl %ecx, %eax\n",
            "    movq %rdx, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_cfcmovl_definer() {
        // APX `cfcmov` is conditional like `cmov`: the dest keeps its old
        // (unknown) upper half on the untaken path. Future-proofing — no
        // current CPU emits it, but the classifier must already refuse it.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cfcmovae %eax, %r10d\n",
            "    movl %r10d, %edx\n",
            "    orq %rdx, %rdi\n",
            "    movq %rdi, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %edx"), "{out}");
        assert!(out.contains("orq %rdx, %rdi"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_live_source() {
        // %r8d is read past a barrier: the rename window ends cleanly, but
        // the source's old value is live, so the candidate must be refused.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    orl %r10d, %edi\n",
            "    jmp .Lx\n",
            ".Lx:\n",
            "    addl %r8d, %eax\n",
            "    movl %edi, %edx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_source_read_inside_the_window() {
        // The source is read between the consumer and the last dest use: the
        // rename window must abort (a pre-existing read would observe the
        // consumer's new value instead of the old one).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    orl %r10d, %edi\n",
            "    addl %r8d, %eax\n",
            "    movl %edi, %edx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_renames_rmw_of_dest_inside_the_window() {
        // A read-modify-write of %D renames like a read: %S holds %D's
        // current value (the retargeted consumer established it), so `addl
        // %eax, %r8d` computes the identical result+flags from the same
        // values, and the later %D read (renamed to %S) observes exactly
        // the value it would have. (`xchg`/`cmov`/partial writes still
        // abort — unmeasured, conservatively kept.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    addl %eax, %r10d\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $2080895, %r8d"), "{out}");
        assert!(out.contains("addl %eax, %r8d"), "{out}");
        assert!(out.contains("movl %r8d, %eax"), "{out}");
        assert!(!out.contains("%r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_dest_live_at_return() {
        // %rax is read implicitly by `ret`: the dest is not dead after the
        // rewrite, so the transform must roll back.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r10d, %eax\n",
            "    andl $2080895, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r10d, %eax"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_xchg_of_dest_inside_the_window() {
        // `xchg` reads AND writes both operands: renaming it would write the
        // source family mid-window and corrupt the later renamed reads.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    xchgl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_cmov_of_dest_inside_the_window() {
        // A conditional write of %r10d is still a write: renaming it would
        // land its result in the wrong register on the taken path.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl $2080895, %r10d\n",
            "    cmovzl %eax, %r10d\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
    }

    #[test]
    fn relay_is_kept_when_a_call_reads_the_target_as_an_argument() {
        // %rdi is never mentioned again textually, but `call bar` reads it as
        // the first SysV argument: whole-function uniqueness must not conclude
        // the copy is dead.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rax, %rdi\n",
            "    addq %rdi, %rsi\n",
            "    call bar\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rax, %rdi"), "{out}");
    }

    #[test]
    fn relay_is_kept_when_a_call_reads_the_static_chain() {
        // %r10 carries the static chain of a nested-function call. Only
        // chain calls read it, and they are marked `# LCCC_CHAIN_CALL`
        // right after the call text (calls.rs). Without the marker this
        // call would be a plain scratch-clobbering call and the staging
        // copy would be foldable -- the marker is what makes the
        // ABI-invisible read visible to the text liveness oracle, so the
        // `movq %rbx, %r10` staging must survive: the callee's chain
        // depends on it.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rbx, %r10\n",
            "    addq %r10, %rsi\n",
            "    call nested.0\n",
            "    # LCCC_CHAIN_CALL\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rbx, %r10"), "{out}");
    }

    #[test]
    fn relay_is_kept_when_ret_returns_the_target() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    addl %eax, %esi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %ebx, %eax"), "{out}");
    }

    #[test]
    fn producer_is_retargeted_into_the_copy_destination() {
        // The store keeps %r10 alive, so the copy cannot simply be deleted;
        // the producer must write %r10d directly instead.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rdx,%r12), %eax\n",
            "    movq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    movl $7, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Either the producer writes %r10d directly, or (better) the store
        // relay reads %rax and the copy disappears entirely. Both are correct;
        // what must NOT happen is keeping the copy.
        assert!(!out.contains("movq %rax, %r10"), "{out}");
        assert!(
            out.contains("movzbl (%rdx,%r12), %r10d") || out.contains("movq %rax, (%rsi)"),
            "{out}"
        );
    }

    #[test]
    fn producer_retarget_allows_sources_naming_either_register() {
        // The return-value tail: the zero-extend reads %al and the copy
        // writes %rax. x86 reads the source before the write.
        let ret = |prod: &str| {
            run(&format!(
                "foo:\n.cfi_startproc\n    # LCCC_RET_XMM 0\n    # LCCC_RET_RDX 0\n    # LCCC_RET_RAX 1\n    addq %rsi, %rax\n    {prod}\n    movq %rdx, %rax\n    ret\n.cfi_endproc\n"
            ))
        };
        let out = ret("movzbl %al, %edx");
        assert!(
            out.contains("movzbl %al, %eax") && !out.contains("%rdx"),
            "{out}"
        );
        // The producer reads its own register (dead after the copy).
        let out = ret("movzbl %dl, %edx");
        assert!(
            out.contains("movzbl %dl, %eax") && !out.contains("movq %rdx"),
            "{out}"
        );
        // A 64-bit round trip is the identity: both lines go.
        let out = ret("movq %rax, %rdx");
        assert!(
            !out.contains("%rdx") && !out.contains("movq %rax, %rax"),
            "{out}"
        );
        // A memory source whose address names the destination.
        let out = ret("movl 8(%rax,%rdx), %edx");
        assert!(
            out.contains("movl 8(%rax,%rdx), %eax") && !out.contains("movq %rdx"),
            "{out}"
        );
        // The producer's register stays live: no retarget.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl %al, %edx\n",
            "    movq %rdx, %rax\n",
            "    movq %rdx, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl %al, %edx"), "{out}");
    }

    #[test]
    fn producer_retarget_respects_a_live_source() {
        // %eax is read after the copy: the producer must keep writing %eax.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rdx), %eax\n",
            "    movq %rax, %r10\n",
            "    movq %r10, (%rsi)\n",
            "    addl %eax, %ecx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl (%rdx), %eax"), "{out}");
    }

    #[test]
    fn wide_producer_under_narrow_copy_is_rejected() {
        // movq writes 64 bits; the movl copy keeps only the low half, so the
        // producer may not be retargeted (the upper half would survive).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq (%rdx), %rax\n",
            "    movl %eax, %r10d\n",
            "    movq %r10, (%rsi)\n",
            "    movq $0, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq (%rdx), %rax"), "{out}");
        assert!(out.contains("movl %eax, %r10d"), "{out}");
    }

    #[test]
    fn windowed_lea_is_folded_into_a_later_load() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    movzbl (%rbx), %r13d\n",
            "    addl %r13d, %edi\n",
            "    movzbl (%r10), %r14d\n",
            "    leaq 4(%rbx), %r10\n",
            "    movzbl (%r10), %r15d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl 1(%rbx), %r14d"), "{out}");
        assert!(!out.contains("leaq 1(%rbx)"), "{out}");
    }

    #[test]
    fn rip_relative_lea_folds_into_bare_memory_use() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq gv(%rip), %rcx\n",
            "    movq (%rcx), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq gv(%rip), %rax"), "{out}");
        assert!(!out.contains("leaq gv(%rip)"), "{out}");
    }

    #[test]
    fn rip_relative_lea_never_folds_into_an_indexed_use() {
        // `%rip` addressing has no SIB: splicing an index would make the
        // assembler silently drop it (wrong code), so the LEA must survive.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq V(%rip), %rcx\n",
            "    movq (%rcx, %rsi, 8), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq V(%rip), %rcx"), "{out}");
        assert!(out.contains("movq (%rcx, %rsi, 8), %rax"), "{out}");
    }

    #[test]
    fn register_lea_folds_into_an_indexed_use() {
        // One base register + one index = a legal SIB.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq (%rbx), %rcx\n",
            "    movq (%rcx, %rsi, 8), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq (%rbx, %rsi, 8), %rax"), "{out}");
        assert!(!out.contains("leaq (%rbx)"), "{out}");
    }

    #[test]
    fn two_register_lea_never_folds_into_an_indexed_use() {
        // Base+index (2 regs) folded into another indexed use would need a
        // third register slot: invalid SIB (`(%rbx, %rdi, %rsi, 8)`).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq (%rbx, %rdi), %rcx\n",
            "    movq (%rcx, %rsi, 8), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq (%rbx, %rdi), %rcx"), "{out}");
        assert!(out.contains("movq (%rcx, %rsi, 8), %rax"), "{out}");
    }

    #[test]
    fn lea_is_kept_when_the_temporary_survives_the_use() {
        // Pins `the refusal under test; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    movzbl (%r10), %r14d\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %r10"), "{out}");
    }

    #[test]
    fn lea_is_kept_when_the_base_is_redefined() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    addq $8, %rbx\n",
            "    movzbl (%r10), %r14d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %r10"), "{out}");
    }

    /// A displacement-form use absorbs the LEA by SUMMING the displacements.
    /// It used to be rejected outright because the only available rewrite was
    /// a textual splice, which would have produced `movzbl 81(%rbx)`.
    #[test]
    fn displacement_form_use_composes_displacements() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    movzbl 8(%r10), %r14d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl 9(%rbx), %r14d"), "{out}");
        assert!(!out.contains("81(%rbx)"), "{out}");
        assert!(!out.contains("leaq 1(%rbx), %r10"), "{out}");
    }

    /// The loop-latch increment: the copy dest is live across the back edge
    /// and the producer reg feeds the exit cmp, so neither relay pass fires.
    /// The LEA must be retargeted onto its own base and the cmp renamed.
    #[test]
    fn latch_increment_folds_into_lea_base() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %esi, %edx\n",
            "    xorl %r8d, %r8d\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpq %rdx, %rbx\n",
            "    jae .LBB3\n",
            ".LBB2:\n",
            "    movslq (%rdi,%rbx,4), %r9\n",
            "    leaq (%r8,%r9,1), %r8\n",
            "    leaq 1(%rbx), %r11\n",
            "    movq %r11, %rbx\n",
            "    cmpq %rdx, %r11\n",
            "    jb .LBB2\n",
            ".LBB3:\n",
            "    movq %r8, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %rbx"), "{out}");
        assert!(out.contains("cmpq %rdx, %rbx"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    /// The real `a[i>>1]` accumulate shape: the producer family `%r11` is
    /// ALSO used by the shift copy at the top of the loop, so neither
    /// syntactic proof applies and only the `FileLiveness` dataflow answer
    /// can settle the fold. Pins the proof anchor at the LEA line: querying
    /// the (NOP-marked) copy's index would answer `None` — FileLiveness only
    /// marks real instructions as known — and silently decline the fold.
    #[test]
    fn latch_fold_settled_by_dataflow_when_family_reused_in_loop() {
        let out = run(concat!(
            "shift_half:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    .cfi_def_cfa_offset 16\n",
            "    movl %esi, %edx\n",
            "    xorl %r8d, %r8d\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpq %rdx, %rbx\n",
            "jae .LBB3\n",
            ".LBB2:\n",
            "    movq %rbx, %r11\n",
            "    shrq $1, %r11\n",
            "    movslq (%rdi, %r11, 4), %r9\n",
            "    leaq (%r8, %r9, 1), %r8\n",
            "    leaq 1(%rbx), %r11\n",
            "    movq %r11, %rbx\n",
            "    cmpq %rdx, %r11\n",
            "jb .LBB2\n",
            ".LBB3:\n",
            "    movq %r8, %rax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %rbx"), "{out}");
        assert!(out.contains("cmpq %rdx, %rbx"), "{out}");
        // The shift copy legitimately keeps %r11 (shr is destructive).
        assert!(out.contains("movq %rbx, %r11"), "{out}");
    }

    #[test]
    fn lea_base_fold_crosses_independent_same_block_work() {
        // A small scalar loop can compute its next induction value, do
        // independent accumulation work, then commit the value to the base.
        // The LEA can write the base directly because neither the base nor
        // its temporary is touched in the intervening instructions.
        let (out, changed) = run_lea_base_fold(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB1:\n",
            "    leaq -1(%r9), %rax\n",
            "    movzbl (%rdi), %esi\n",
            "    leal (%r14, %rsi), %r10d\n",
            "    addl %r10d, %r12d\n",
            "    movq %rax, %r9\n",
            "    testq %r9, %r9\n",
            "    jne .LBB1\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(changed, "the safe gap should be folded:\n{out}");
        assert!(out.contains("leaq -1(%r9), %r9"), "{out}");
        assert!(!out.contains("movq %rax, %r9"), "{out}");
    }

    #[test]
    fn lea_base_fold_does_not_cross_intervening_base_read() {
        // Retargeting the LEA early would change this intervening read, so
        // only instructions independent of both register families may be
        // crossed.
        let (out, changed) = run_lea_base_fold(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB1:\n",
            "    leaq -1(%r9), %rax\n",
            "    movq %r9, %r10\n",
            "    movq %rax, %r9\n",
            "    testq %r9, %r9\n",
            "    jne .LBB1\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            !changed,
            "a base read in the gap must block the fold:\n{out}"
        );
        assert!(out.contains("leaq -1(%r9), %rax"), "{out}");
        assert!(out.contains("movq %rax, %r9"), "{out}");
    }

    /// 32-bit latch: same fold at `leal`/`movl` width.
    #[test]
    fn latch_increment_folds_at_32_bit_width() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    movslq (%rdi,%rbx,4), %r9d\n",
            "    leal 1(%ebx), %r10d\n",
            "    movl %r10d, %ebx\n",
            "    cmpl %esi, %r10d\n",
            "    jb .LBB1\n",
            "    movl %ebx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal 1(%ebx), %ebx"), "{out}");
        assert!(!out.contains("%r10d"), "{out}");
    }

    /// The producer register is read AFTER the loop: the rename window ends
    /// at the conditional jump, the deadness proof must fail, and the whole
    /// candidate must roll back (no partial rewrite may survive).
    #[test]
    fn lea_base_fold_rolls_back_when_producer_live_out() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    leaq 1(%rbx), %r11\n",
            "    movq %r11, %rbx\n",
            "    cmpq %rdx, %r11\n",
            "    jb .LBB1\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %r11"), "{out}");
        assert!(out.contains("movq %r11, %rbx"), "{out}");
        assert!(out.contains("movl %r11d, %eax"), "{out}");
    }

    /// A store of the producer register inside the window is renamed with
    /// the same value-equivalence argument as the cmp.
    #[test]
    fn lea_base_fold_renames_store_of_producer() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 8(%rbx), %r11\n",
            "    movq %r11, %rbx\n",
            "    movq %r11, (%r13)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 8(%rbx), %rbx"), "{out}");
        assert!(out.contains("movq %rbx, (%r13)"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    /// A read-modify-write of the producer register inside the window would
    /// land its result in the wrong register after a rename: decline.
    #[test]
    fn lea_base_fold_declines_rmw_of_producer() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    leaq 1(%rbx), %r11\n",
            "    movq %r11, %rbx\n",
            "    addq %r11, %r11\n",
            "    cmpq %rdx, %r11\n",
            "    jb .LBB1\n",
            "    movl %ebx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %r11"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_shift_count_of_dest() {
        // `shll %cl, %ecx`: the count register is FIXED by the ISA, so the
        // destination cannot be retargeted — renaming `%cl` emits invalid
        // x86 (`shll %r8b, ...`), and keeping it reads the stale pre-copy
        // count. No sound rewrite exists; the pair must stay verbatim.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %ecx\n",
            "    shll %cl, %ecx\n",
            "    movl %ecx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %ecx"), "{out}");
        assert!(out.contains("shll %cl, %ecx"), "{out}");
        assert!(!out.contains("%r8b"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_shift_count_read_inside_the_window() {
        // A `%cl` read of the destination inside the rename window is the
        // same hazard one line later: the count cannot be renamed (invalid
        // x86) and cannot be kept (stale value). The window must abort.
        // (`andl`-immediate consumer: the wide mask defeats `copy_mask_movz`
        // so the pair reaches this pass.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %ecx\n",
            "    andl $2080895, %ecx\n",
            "    shll %cl, %eax\n",
            "    movl %eax, %edx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $2080895, %ecx"), "{out}");
        assert!(out.contains("shll %cl, %eax"), "{out}");
        assert!(!out.contains("%r8b"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_shift_by_cl_of_other_family() {
        // `%cl` is only special when it names the COALESCED family: here the
        // count belongs to `%rcx` while the copy moves `%r8d`→`%r10d`, so
        // the consumer retargets and the count survives untouched.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shll %cl, %r10d\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("shll %cl, %r8d"), "{out}");
        assert!(out.contains("movl %r8d, %eax"), "{out}");
        assert!(!out.contains("%r10"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_source_read_by_consumer() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // The consumer reads the source (`andl %r8d, %r10d`): the source's
        // old value is live into the consumer, so the clobber cannot be
        // proven dead and the candidate must be refused. (The rename
        // window's `%S` scan is NOT for this case — it guards reads of a
        // REDEFINED source further down the window.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    andl %r8d, %r10d\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("andl %r8d, %r10d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_renames_byte_read_of_shift_count_family() {
        // The `%cl` refusal is shift/rotate-specific: as an ordinary byte
        // operand (`movzbl %cl, %eax`) `%cl` renames to `%r8b` soundly —
        // any byte register is a valid, value-identical spelling there.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %ecx\n",
            "    orl $3, %ecx\n",
            "    movzbl %cl, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("orl $3, %r8d"), "{out}");
        assert!(out.contains("movzbl %r8b, %eax"), "{out}");
        assert!(!out.contains("%ecx"), "{out}");
        assert!(!out.contains("%cl"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_zext_copy_under_masked_consumer() {
        // The sqlite-varint exit shape: `movzbl %dl, %r11d` is not a plain
        // copy (the upper bits differ from %edx's), but the `andl $127`
        // consumer clears every bit the two spellings could disagree on —
        // so the pair folds exactly like `movl %edx, %r11d`, and the
        // following shift (an RMW of the destination) renames with it.
        // (The `# LCCC_RET_RDX 0` line models prologue output: without the
        // return-type fact the `%rdx` mention defeats the tail-block
        // detector and the source proof conservatively refuses.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    movzbl %dl, %r11d\n",
            "    andl $127, %r11d\n",
            "    shll $7, %r11d\n",
            "    movzbl %r9b, %r10d\n",
            "    orl %r11d, %r10d\n",
            "    movq %r10, (%rsi)\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $127, %edx"), "{out}");
        assert!(out.contains("shll $7, %edx"), "{out}");
        assert!(out.contains("orl %edx, %r10d"), "{out}");
        assert!(!out.contains("%r11d"), "{out}");
        assert!(!out.contains("movzbl %dl"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_zext_copy_with_non_return_source() {
        // Control for the masked-zext fold: the source family here is
        // `%r9` (never read by `ret`), so the source-old-value proof needs
        // no return-type facts and the pair folds.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl %r9b, %r11d\n",
            "    andl $127, %r11d\n",
            "    shll $7, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $127, %r9d"), "{out}");
        assert!(out.contains("shll $7, %r9d"), "{out}");
        assert!(out.contains("movl %r9d, %eax"), "{out}");
        assert!(!out.contains("%r11d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_zext_when_rdx_return_marker_set() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // The sound direction of the return-type fact: `# LCCC_RET_RDX 1`
        // (an i128 return) keeps `%rdx` live at `ret`, so the source's old
        // value cannot be proven dead and the pair must stay.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 1\n",
            "    movzbl %dl, %r11d\n",
            "    andl $127, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl %dl, %r11d"), "{out}");
        assert!(out.contains("andl $127, %r11d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_zext_copy_under_wide_mask() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // `$511` keeps bit 8, where the zero-extension (0) and the raw
        // source (garbage) disagree: folding would compute the wrong value.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl %dl, %r11d\n",
            "    andl $511, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl %dl, %r11d"), "{out}");
        assert!(out.contains("andl $511, %r11d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_refuses_zext_copy_under_non_mask_consumer() {
        // Pins `coalesce_copy_into_rmw`'s refusal; the late chain rename
        // (own tests: `chain_rename_tests`) would rename the shape.
        let _skip = super::super::test_skips::skip_passes(&["chain_retarget"]);
        // `orl` does not kill the extension garbage (it ORs it in): only an
        // `andl` with a subsuming mask admits an extension copy.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl %dl, %r11d\n",
            "    orl $127, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl %dl, %r11d"), "{out}");
        assert!(out.contains("orl $127, %r11d"), "{out}");
    }

    #[test]
    fn rmw_coalesce_folds_word_zext_copy_under_masked_consumer() {
        // The 16-bit form: `movswl %dx, %r11d` agrees with %edx on the low
        // 16 bits, and `$65535` clears the rest.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    # LCCC_RET_RDX 0\n",
            "    movswl %dx, %r11d\n",
            "    andl $65535, %r11d\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $65535, %edx"), "{out}");
        assert!(out.contains("movl %edx, %eax"), "{out}");
        assert!(!out.contains("%r11d"), "{out}");
    }

    // ------------------------------------------------------------------
    // SibAddr parsing and compose_sib: the fold that miscompiled
    // div/mod-by-constant verification at -O1 lived here.  Every case below
    // is a shape the fold can actually meet in emitted code.
    // ------------------------------------------------------------------

    #[test]
    fn sib_parse_index_only_scale4() {
        let a = SibAddr::parse("0(,%r11,4)").expect("index-only lea parses");
        assert_eq!(a.disp, 0);
        assert_eq!(a.base, None);
        assert_eq!(a.index, Some(("%r11".to_string(), 4)));
    }

    #[test]
    fn sib_parse_base_and_index() {
        let a = SibAddr::parse("(%r8, %r10, 1)").expect("base+index parses");
        assert_eq!(a.base.as_deref(), Some("%r8"));
        assert_eq!(a.index, Some(("%r10".to_string(), 1)));
        assert_eq!(a.disp, 0);
    }

    #[test]
    fn sib_parse_disp_base_index() {
        let a = SibAddr::parse("8(%rcx,%rbp,8)").expect("disp+base+index parses");
        assert_eq!(a.disp, 8);
        assert_eq!(a.base.as_deref(), Some("%rcx"));
        assert_eq!(a.index, Some(("%rbp".to_string(), 8)));
    }

    #[test]
    fn sib_parse_base_only_renders_canonically() {
        let a = SibAddr::parse("16(%rax)").unwrap();
        assert_eq!(a.base.as_deref(), Some("%rax"));
        assert_eq!(a.index, None);
        assert_eq!(a.render(), "16(%rax)");
        let b = SibAddr::parse("(%rax)").unwrap();
        assert_eq!(b.render(), "(%rax)", "zero disp renders without it");
    }

    #[test]
    fn sib_parse_rejects_unencodable_forms() {
        // %rip-relative: ModRM mod=00 r/m=101, no SIB byte at all.
        assert!(SibAddr::parse("V(%rip)").is_none());
        assert!(SibAddr::parse(".Lstr0(%rip)").is_none());
        // Illegal index scale.
        assert!(SibAddr::parse("(,%r11,3)").is_none());
        // Three register slots: no encoding exists.
        assert!(SibAddr::parse("(%rax,%rbx,%rcx)").is_none());
        // No operand at all.
        assert!(SibAddr::parse("()").is_none());
        assert!(SibAddr::parse("%rax").is_none());
    }

    /// THE regression: `leaq 0(,%r11,4), %r8` folded into
    /// `leaq (%r8, %r10, 1), %r9`.  The old code concatenated the two operand
    /// texts into `0(,%r11,4, %r10, 1)` — two index registers, no base, no
    /// encoding; the integrated assembler silently dropped `+ %r10` and the
    /// `q*d + r == n` check compiled to `q*d == n`.
    #[test]
    fn compose_index_only_producer_into_indexed_consumer() {
        let inner = SibAddr::parse("0(,%r11,4)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%r10".to_string(), 1)),
        };
        let got = compose_sib(&inner, &outer).expect("must compose");
        // disp 0 is omitted: `(%r10, %r11, 4)` is the canonical GAS form.
        assert_eq!(got, "(%r10, %r11, 4)");
        // And the result must itself be a legal, single-index operand.
        let reparsed = SibAddr::parse(&got).expect("composition re-parses");
        assert_eq!(reparsed.base.as_deref(), Some("%r10"));
        assert_eq!(reparsed.index, Some(("%r11".to_string(), 4)));
    }

    #[test]
    fn compose_plain_base_producer_keeps_working() {
        // The shape the fold was originally written for: a plain
        // displacement(base) producer folded into an indexed consumer.
        let inner = SibAddr::parse("16(%rax)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 8,
            base: None,
            index: Some(("%rbx".to_string(), 8)),
        };
        assert_eq!(
            compose_sib(&inner, &outer).as_deref(),
            Some("24(%rax, %rbx, 8)")
        );
    }

    #[test]
    fn compose_rejects_two_scaled_registers() {
        // 4*%rax + 8*%rbx needs two index slots.
        let inner = SibAddr::parse("(,%rax,4)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%rbx".to_string(), 8)),
        };
        assert!(compose_sib(&inner, &outer).is_none());
    }

    #[test]
    fn compose_rejects_three_distinct_registers() {
        let inner = SibAddr::parse("(%rax,%rbx,2)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%rcx".to_string(), 1)),
        };
        assert!(compose_sib(&inner, &outer).is_none());
    }

    #[test]
    fn compose_merges_repeated_register_into_one_scale() {
        // %rax + %rax*1 == 2*%rax, which is a legal index scale.
        let inner = SibAddr::parse("(%rax)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%rax".to_string(), 1)),
        };
        assert_eq!(compose_sib(&inner, &outer).as_deref(), Some("(,%rax, 2)"));
    }

    #[test]
    fn compose_rejects_unmergeable_repeated_register() {
        // %rax*1 + %rax*2 == 3*%rax: not an encodable scale.
        let inner = SibAddr::parse("(,%rax,2)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%rax".to_string(), 1)),
        };
        assert!(compose_sib(&inner, &outer).is_none());
    }

    #[test]
    fn compose_sums_displacements() {
        let inner = SibAddr::parse("-8(%rax)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 8,
            base: None,
            index: None,
        };
        assert_eq!(compose_sib(&inner, &outer).as_deref(), Some("(%rax)"));
    }

    /// End-to-end through the real peephole driver: the exact instruction
    /// pair from the -O1 div/mod-by-constant miscompile.  The fold must
    /// produce ONE lea with a single index register; a two-index operand has
    /// no x86-64 encoding and used to assemble into wrong code.
    #[test]
    fn lea_chain_with_index_producer_composes_not_concatenates() {
        let out = run(concat!(
            "d4:\n",
            ".cfi_startproc\n",
            "    leaq 0(,%r11,4), %r8\n",
            "    leaq (%r8, %r10, 1), %r9\n",
            "    cmpq %rbx, %r9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            !out.contains("%r11,4, %r10"),
            "two-index lea escaped into the output:\n{out}"
        );
        assert!(
            !out.contains("%r11, 4, %r10"),
            "two-index lea escaped into the output:\n{out}"
        );
        // Either the fold fired and produced the valid composed form, or it
        // declined and left both instructions intact.  Both are correct; the
        // concatenation is the only unacceptable answer.
        if out.contains("leaq (%r10, %r11, 4), %r9") {
            assert!(
                !out.contains("leaq 0(,%r11,4), %r8"),
                "producer not removed"
            );
        } else {
            assert!(out.contains("leaq 0(,%r11,4), %r8"), "{out}");
            assert!(out.contains("leaq (%r8, %r10, 1), %r9"), "{out}");
        }
    }

    // -- symbolic displacements and operand position ----------------------

    #[test]
    fn sib_parse_symbolic_displacements() {
        let a = SibAddr::parse("table(%rcx)").unwrap();
        assert_eq!(a.sym.as_deref(), Some("table"));
        assert_eq!(a.disp, 0);
        assert_eq!(a.base.as_deref(), Some("%rcx"));
        assert_eq!(a.render(), "table(%rcx)");

        let b = SibAddr::parse("table+8(%rax)").unwrap();
        assert_eq!(b.sym.as_deref(), Some("table"));
        assert_eq!(b.disp, 8);
        assert_eq!(b.render(), "table+8(%rax)");

        let c = SibAddr::parse("table-4(%rax)").unwrap();
        assert_eq!(c.sym.as_deref(), Some("table"));
        assert_eq!(c.disp, -4);
        assert_eq!(c.render(), "table-4(%rax)");
    }

    /// The indexed-relay fold must keep working for a symbol-based producer:
    /// this is the `leaq table(%rcx), %r8; movq (%r8,%rbp,8), %r13` shape the
    /// pass documents (hash_table, adler32 unrolled loops).  A symbol cannot
    /// be added, so it is carried through verbatim.
    #[test]
    fn compose_symbolic_producer_into_indexed_consumer() {
        let inner = SibAddr::parse("table(%rcx)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 8,
            base: None,
            index: Some(("%rbp".to_string(), 8)),
        };
        assert_eq!(
            compose_sib(&inner, &outer).as_deref(),
            Some("table+8(%rcx, %rbp, 8)")
        );
    }

    #[test]
    fn compose_rejects_two_symbolic_displacements() {
        let inner = SibAddr::parse("table(%rcx)").unwrap();
        let outer = SibAddr {
            sym: Some("other".to_string()),
            disp: 0,
            base: None,
            index: Some(("%rbp".to_string(), 8)),
        };
        assert!(compose_sib(&inner, &outer).is_none());
    }

    #[test]
    fn operand_start_finds_first_and_later_operands() {
        // First operand: just past the mnemonic.
        assert_eq!(operand_start("leaq (%r8, %r10, 1), %r9", 5), 4);
        // Second operand (a store): just past the separator comma, NOT just
        // past the mnemonic -- otherwise the displacement parse would choke
        // on `%rax,` and the fold would silently stop firing for stores.
        let store = "movq %rax, (%rcx,%rbp,8)";
        assert_eq!(operand_start(store, 11), 10);
        assert_eq!(&store[11..], "(%rcx,%rbp,8)");
    }

    /// The store form must still fold: a register operand precedes the
    /// memory operand and the displacement extractor has to step over it.
    #[test]
    fn indexed_store_still_folds() {
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    leaq 16(%r12), %r8\n",
            "    movq %rax, (%r8,%rbp,8)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq %rax, 16(%r12, %rbp, 8)"),
            "store into an indexed operand stopped folding:\n{out}"
        );
    }

    #[test]
    fn render_omits_implicit_scale_one() {
        // The canonical AT&T form leaves scale 1 implicit.  The old splicer
        // produced `8(%rsp, %r8)`; a renderer that wrote `8(%rsp, %r8, 1)`
        // would be equivalent but would change every folded operand and break
        // downstream pattern matchers (dead_writes keys on the canonical form).
        let a = SibAddr::parse("(%rcx, %r8)").unwrap();
        assert_eq!(a.index, Some(("%r8".to_string(), 1)));
        assert_eq!(a.render(), "(%rcx, %r8)");
        // And a scale-1 result still composes into the canonical text.
        let inner = SibAddr::parse("8(%rsp)").unwrap();
        let outer = SibAddr {
            sym: None,
            disp: 0,
            base: None,
            index: Some(("%r8".to_string(), 1)),
        };
        assert_eq!(compose_sib(&inner, &outer).as_deref(), Some("8(%rsp, %r8)"));
    }

    #[test]
    fn parse_disp_handles_hex_literals() {
        // A hex displacement must be a NUMBER, not a symbol: as a symbol it
        // would silently disable displacement arithmetic in compose_sib.
        assert_eq!(parse_disp("0x10"), Some((None, 16)));
        assert_eq!(parse_disp("0X10"), Some((None, 16)));
        assert_eq!(parse_disp("0x4"), Some((None, 4)));
        assert_eq!(parse_disp(""), Some((None, 0)));
        assert_eq!(parse_disp("16"), Some((None, 16)));
        assert_eq!(parse_disp("-8"), Some((None, -8)));
        // Unsigned reinterpretation: the all-ones word is -1 as a bit pattern.
        assert_eq!(parse_disp("0xffffffffffffffff"), Some((None, -1)));
        // Negated hex reaches the full range without the reinterpretation.
        assert_eq!(parse_disp("-0x10"), Some((None, -16)));
        // Symbols and symbol+offset still parse as symbols.
        assert_eq!(parse_disp("table"), Some((Some("table".to_string()), 0)));
        assert_eq!(parse_disp("table+8"), Some((Some("table".to_string()), 8)));
        assert_eq!(parse_disp("table-4"), Some((Some("table".to_string()), -4)));
        // A bare sign is not a displacement.
        assert_eq!(parse_disp("+"), None);
    }

    /// Hex displacements feed the composition arithmetic the same way decimal
    /// ones do, so a hex-addressed producer still folds.
    #[test]
    fn compose_sums_hex_displacements() {
        let inner = SibAddr::parse("0x10(%rax)").expect("hex disp parses to a number");
        assert_eq!(inner.disp, 16);
        assert_eq!(inner.sym, None, "a hex literal is not a symbol");
        let outer = SibAddr {
            sym: None,
            disp: 8,
            base: None,
            index: None,
        };
        assert_eq!(compose_sib(&inner, &outer).as_deref(), Some("24(%rax)"));
    }

    /// sqlite3GenerateColumnNames at -O1: `pEList->a[i].zEName` was loaded
    /// through `88(%r13,%r10)` instead of `16(%r13,%r10)` because the
    /// consumer's `8` was glued in front of the producer's `8(`.
    #[test]
    fn splice_sums_consumer_and_producer_displacements() {
        assert_eq!(
            splice_lea_into_mem_operand("movq 8(%r9), %rcx", "%r9", "8(%r13, %r10)").as_deref(),
            Some("movq 16(%r13, %r10), %rcx")
        );
        assert_eq!(
            splice_lea_into_mem_operand("movq (%r9), %rcx", "%r9", "8(%r13,%r10)").as_deref(),
            Some("movq 8(%r13, %r10), %rcx")
        );
        // Store destination operand, negative and hex displacements.
        assert_eq!(
            splice_lea_into_mem_operand("movl %eax, -4(%rdx)", "%rdx", "0x10(%rsi)").as_deref(),
            Some("movl %eax, 12(%rsi)")
        );
        // Opposite displacements cancel to GAS's canonical empty form.
        assert_eq!(
            splice_lea_into_mem_operand("movzbl -8(%rdx), %eax", "%rdx", "8(%rsi,%rcx,4)")
                .as_deref(),
            Some("movzbl (%rsi, %rcx, 4), %eax")
        );
        // A consumer index composes with a single-base producer.
        assert_eq!(
            splice_lea_into_mem_operand("movq 8(%rdx,%rcx,8), %rax", "%rdx", "16(%rsi)").as_deref(),
            Some("movq 24(%rsi, %rcx, 8), %rax")
        );
        // A symbolic producer displacement survives verbatim.
        assert_eq!(
            splice_lea_into_mem_operand("movl 4(%rdx), %eax", "%rdx", "table(%rsi)").as_deref(),
            Some("movl table+4(%rsi), %eax")
        );
    }

    #[test]
    fn splice_fails_closed_on_unrepresentable_or_ambiguous_operands() {
        // Three registers cannot share one SIB byte.
        assert_eq!(
            splice_lea_into_mem_operand("movq (%rdx,%rcx,8), %rax", "%rdx", "(%rsi,%rdi)"),
            None
        );
        // %T in the INDEX slot is not a base substitution.
        assert_eq!(
            splice_lea_into_mem_operand("movq (%rsi,%rdx,8), %rax", "%rdx", "8(%rdi)"),
            None
        );
        // Two memory operands based on %T (movs-style) are not half-rewritten.
        assert_eq!(
            splice_lea_into_mem_operand("cmpsb (%rdx), (%rdx)", "%rdx", "8(%rdi)"),
            None
        );
        // A *segment override* is the one operand prefix that IS composed
        // through: it is stripped for the address parse and re-attached in
        // front of the composed displacement (GCC's per-CPU shape,
        // `leaq 8(%rdi), %rdx; movq %fs:(%rdx), %rax` -> `movq %fs:8(%rdi), %rax`).
        // A prefix that is not one of the six segment registers stays refused.
        assert_eq!(
            splice_lea_into_mem_operand("movq %fs:8(%rdx), %rax", "%rdx", "8(%rdi)").as_deref(),
            Some("movq %fs:16(%rdi), %rax")
        );
        assert_eq!(
            splice_lea_into_mem_operand("movq %gss:8(%rdx), %rax", "%rdx", "8(%rdi)"),
            None
        );
        assert_eq!(
            splice_lea_into_mem_operand("jmp *8(%rdx)", "%rdx", "8(%rdi)"),
            None
        );
        // Two symbols cannot be added.
        assert_eq!(
            splice_lea_into_mem_operand("movl a(%rdx), %eax", "%rdx", "b(%rsi)"),
            None
        );
        // disp32 overflow has no encoding.
        assert_eq!(
            splice_lea_into_mem_operand("movl 2147483647(%rdx), %eax", "%rdx", "1(%rsi)"),
            None
        );
        // `%r1` must not match inside `%r10`.
        assert_eq!(
            splice_lea_into_mem_operand("movq 8(%r10), %rax", "%r1", "8(%rsi)"),
            None
        );
        // A consumer without the register at all.
        assert_eq!(
            splice_lea_into_mem_operand("movq %rdx, %rax", "%rdx", "8(%rsi)"),
            None
        );
    }

    #[test]
    fn parse_disp_rejects_operand_prefixes_as_symbols() {
        assert_eq!(parse_disp("%fs:8"), None);
        assert_eq!(parse_disp("*8"), None);
        assert_eq!(parse_disp("lock addq 8"), None);
        assert_eq!(parse_disp(".LC0+8"), Some((Some(".LC0".to_string()), 8)));
        assert_eq!(parse_disp("x.1"), Some((Some("x.1".to_string()), 0)));
    }

    /// The segment-override spelling is recognised as a prefix and nothing
    /// else: exactly two letters from the segment set, one colon, no trailing
    /// text inside the prefix.
    #[test]
    fn segment_prefix_validation_is_exact() {
        use super::segment_prefix_len;
        for ok in ["%gs:", "%fs:", "%es:", "%ds:", "%cs:", "%ss:", "%GS:"] {
            assert_eq!(segment_prefix_len(ok), Some(4), "{ok} is a prefix");
        }
        // The detector answers for the PREFIX only; the residual text after it
        // belongs to the caller (`%gs:8` is a four-byte prefix plus `8`, which
        // is exactly the shape the splice parses).
        assert_eq!(segment_prefix_len("%gs:8"), Some(4));
        assert_eq!(segment_prefix_len("%gs:"), Some(4));
        for bad in [
            "", "%", "%g:", "%gss:", "%gx:", "%xyz:", "%gs8:", "%gs", "gs:", "%%gs:", " %gs:",
            "%$s:", "*%gs:",
        ] {
            assert_eq!(segment_prefix_len(bad), None, "{bad} is not a prefix");
        }
    }

    /// The operand-boundary test the window scan uses: the bare and indexed
    /// matchers must see a consumer behind a segment override, and must still
    /// refuse anything else that puts text before the operand.
    #[test]
    fn segment_override_is_an_operand_boundary_for_the_window_scan() {
        use super::{at_mem_operand_boundary, segment_prefix_len};
        assert!(at_mem_operand_boundary("movq %gs:(%rcx), %rax", 9));
        assert!(at_mem_operand_boundary("movq %gs:(%rcx, %rdx, 8), %rax", 9));
        assert!(at_mem_operand_boundary("movq (%rcx), %rax", 5));
        assert!(at_mem_operand_boundary("(%rcx)", 0));
        // `*8(%rcx)` / `foo:(%rcx)` / `%gsx:(%rcx)` are not operand starts.
        assert!(!at_mem_operand_boundary("jmp *%gs:(%rcx)", 9));
        assert!(!at_mem_operand_boundary("movq 0x64:(%rcx), %rax", 10));
        assert!(!at_mem_operand_boundary("movq %gss:(%rcx), %rax", 10));
        assert_eq!(segment_prefix_len("%gsx:"), None);
    }

    /// GCC and Clang fold a per-CPU load whose base arrives in the accumulator
    /// — `movq %gs:8(%rax), %rax` — and the segment override is the only reason
    /// lccc's window scan used to miss it (the `:` before the `(` was not an
    /// operand boundary).  Reproducer: `long f(void){ return base()->b; }`
    /// (tests/regression/check_seg_const_offset_fold.sh, `fold_after_call`).
    #[test]
    fn lea_folds_into_segment_prefixed_load() {
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    call base@PLT\n",
            "    leaq 8(%rax), %rcx\n",
            "    movq %gs:(%rcx), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq %gs:8(%rax), %rax"),
            "the segment consumer must fold:\n{out}"
        );
        assert!(
            !out.contains("leaq 8(%rax), %rcx"),
            "the lea must be gone:\n{out}"
        );
    }

    /// Same for the indexed consumer (`p[i].field` through a per-CPU array),
    /// which goes through `compose_indexed_relay` — the prefix must survive the
    /// composition and the index/scale must survive with it.
    #[test]
    fn lea_folds_into_indexed_segment_prefixed_load() {
        let out = run(concat!(
            "g:\n",
            ".cfi_startproc\n",
            "    leaq 8(%rax), %rcx\n",
            "    movq %gs:(%rcx, %rdx, 8), %rsi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq %gs:8(%rax, %rdx, 8), %rsi"),
            "the indexed segment consumer must fold:\n{out}"
        );
        assert!(
            !out.contains("leaq 8(%rax), %rcx"),
            "the lea must be gone:\n{out}"
        );
    }

    /// A segment-prefixed *store* destination folds the same way, through the
    /// shared splice: `leaq 8(%rdi), %rcx; movq %rax, %gs:(%rcx)`.
    #[test]
    fn lea_folds_into_segment_prefixed_store() {
        let out = run(concat!(
            "h:\n",
            ".cfi_startproc\n",
            "    leaq 8(%rdi), %rcx\n",
            "    movq %rax, %gs:(%rcx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq %rax, %gs:8(%rdi)"),
            "the segment store must fold:\n{out}"
        );
    }

    /// The fold composes displacements *through* a segment override: both the
    /// producer's and the consumer's displacements survive the rewrite.
    #[test]
    fn segment_prefixed_consumer_composes_displacements() {
        assert_eq!(
            splice_lea_into_mem_operand("movq %gs:(%r9), %rax", "%r9", "8(%r13, %r10)").as_deref(),
            Some("movq %gs:8(%r13, %r10), %rax")
        );
        assert_eq!(
            splice_lea_into_mem_operand("movq %fs:8(%rcx), %rax", "%rcx", "8(%rdi)").as_deref(),
            Some("movq %fs:16(%rdi), %rax")
        );
        assert_eq!(
            splice_lea_into_mem_operand("movl %eax, %gs:(%rdx)", "%rdx", "0x10(%rsi)").as_deref(),
            Some("movl %eax, %gs:16(%rsi)")
        );
        // A prefix that is not a segment register, an indirect branch operand
        // and a doubled prefix all stay refused.
        assert_eq!(
            splice_lea_into_mem_operand("movq %gss:(%r9), %rax", "%r9", "8(%rdi)"),
            None
        );
        assert_eq!(
            splice_lea_into_mem_operand("jmp *%gs:(%r9)", "%r9", "8(%rdi)"),
            None
        );
        assert_eq!(
            splice_lea_into_mem_operand("movq %gs:%fs:(%r9), %rax", "%r9", "8(%rdi)"),
            None
        );
    }
    #[test]
    fn rip_lea_cse_rejects_location_relative_displacements() {
        for displacement in ["8", "0x10", ".", ".+8", ".-8"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    leaq {displacement}(%rip), %r9\n    movq %r9, (%rdi)\n    leaq {displacement}(%rip), %r9\n    movq %r9, (%rsi)\n    ret\n.cfi_endproc\n"
            );
            let mut store = super::LineStore::new(asm.clone());
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| super::classify_line(store.get(i)))
                .collect();
            assert!(!super::cse_symbol_lea(&mut store, &mut infos), "{asm}");
            assert!(infos.iter().all(|info| !info.is_nop()), "{asm}");
        }
        for displacement in ["bins", "bins+8", ".LC0", ".LC0-8"] {
            assert!(
                super::parse_rip_symbol_lea(&format!("leaq {displacement}(%rip), %r9")).is_some(),
                "stable-symbol positive control: {displacement}"
            );
        }
    }
}

/// Tests for the read-modify-write window of `retarget_producer_into_copy`.
///
/// The pass is driven DIRECTLY rather than through `peephole_optimize`:
/// the full pipeline relays the copy into its use before this phase runs,
/// so a pipeline-level fixture can never reach the RMW window and would
/// pass by observing a different transformation entirely.
///
/// `mark_nop` only sets line METADATA (`LineKind::Nop`); the peephole driver
/// blanks the text afterwards. A direct-call test must therefore compare the
/// program built from non-NOP lines, not the raw store text — otherwise a
/// removed copy still reads like a live instruction.
#[cfg(test)]
mod rmw_window_tests {
    /// Run the pass alone over `body`; return the emitted program (every
    /// non-NOP line, trimmed) and how many lines were removed.
    fn run(body: &str) -> (Vec<String>, usize) {
        let asm = format!(".text\nf:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
        let mut store = super::LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        super::retarget_producer_into_copy(&mut store, &mut infos);
        let emitted = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>();
        let removed = (0..store.len()).filter(|&i| infos[i].is_nop()).count();
        (emitted, removed)
    }

    #[test]
    fn same_width_rmw_chain_retargets_the_producer_and_the_rmw() {
        // `%r9`/`%r10`, not `%rax`: `ret` implicitly reads the return
        // register, so an `%eax` producer is never provably dead after the
        // copy and the fold is (correctly) refused -- such a fixture would
        // test nothing.
        let (emitted, removed) = run(concat!(
            "    movl (%rdi), %r9d\n",
            "    addl $1, %r9d\n",
            "    movl %r9d, %r10d\n",
            "    movl %r10d, (%rsi)\n",
            "    ret",
        ));
        assert_eq!(
            emitted,
            vec![
                ".text",
                "f:",
                ".cfi_startproc",
                "movl (%rdi), %r10d",
                "addl $1, %r10d",
                "movl %r10d, (%rsi)",
                "ret",
                ".cfi_endproc",
            ],
            "the producer and its RMW must move to the copy destination and the copy must go"
        );
        assert_eq!(removed, 1, "exactly the copy must be removed");

        // Two-RMW window (`MAX_RMWS = 2`) also retargets both RMWs and drops the copy.
        let (emitted2, removed2) = run(concat!(
            "    movl (%rdi), %r9d\n",
            "    addl $1, %r9d\n",
            "    xorl $3, %r9d\n",
            "    movl %r9d, %r10d\n",
            "    movl %r10d, (%rsi)\n",
            "    ret",
        ));
        assert_eq!(
            emitted2,
            vec![
                ".text",
                "f:",
                ".cfi_startproc",
                "movl (%rdi), %r10d",
                "addl $1, %r10d",
                "xorl $3, %r10d",
                "movl %r10d, (%rsi)",
                "ret",
                ".cfi_endproc",
            ],
            "both RMWs in the 2-RMW window must retarget onto %r10d"
        );
        assert_eq!(removed2, 1, "exactly the copy must be removed");
    }

    #[test]
    fn mismatched_rmw_width_refuses_the_fold() {
        // Identical to the positive control except for the RMW suffix: the
        // width rule must be the ONLY difference between accept and refuse.
        let (emitted, removed) = run(concat!(
            "    movl (%rdi), %r9d\n",
            "    addq $2, %r9\n",
            "    movl %r9d, %r10d\n",
            "    movl %r10d, (%rsi)\n",
            "    ret",
        ));
        assert_eq!(
            emitted,
            vec![
                ".text",
                "f:",
                ".cfi_startproc",
                "movl (%rdi), %r9d",
                "addq $2, %r9",
                "movl %r9d, %r10d",
                "movl %r10d, (%rsi)",
                "ret",
                ".cfi_endproc",
            ],
            "a width-mismatched RMW chain must leave every line in place"
        );
        assert_eq!(removed, 0, "nothing may be removed when the rule refuses");

        // RMW reading the copy destination family (`%r10d`) must also refuse,
        // otherwise retargeting the load into `%r10d` would clobber `%r10d`
        // before `addl %r10d, %r9d` reads it.
        let (emitted_alias, removed_alias) = run(concat!(
            "    movl (%rdi), %r9d\n",
            "    addl %r10d, %r9d\n",
            "    movl %r9d, %r10d\n",
            "    movl %r10d, (%rsi)\n",
            "    ret",
        ));
        assert_eq!(
            emitted_alias,
            vec![
                ".text",
                "f:",
                ".cfi_startproc",
                "movl (%rdi), %r9d",
                "addl %r10d, %r9d",
                "movl %r9d, %r10d",
                "movl %r10d, (%rsi)",
                "ret",
                ".cfi_endproc",
            ],
            "an RMW reading the copy destination family must leave every line in place"
        );
        assert_eq!(removed_alias, 0);
    }

    #[test]
    fn indexed_relay_and_splice_lea_refuse_when_consumer_index_aliases_lea_dst() {
        use super::{compose_indexed_relay, splice_lea_into_mem_operand};
        let line = "movq 8(%rcx,%rcx,8), %rax";
        let pos = line.find('(').unwrap();
        let close = line.find(')').unwrap();
        assert_eq!(
            compose_indexed_relay(line, pos, close, "%rcx", "(%rsi,%rcx,8)"),
            None,
            "compose_indexed_relay must refuse when consumer index shares family with lea dst"
        );
        assert_eq!(
            splice_lea_into_mem_operand(line, "%rcx", "(%rsi,%rcx,8)"),
            None,
            "splice_lea_into_mem_operand must refuse when consumer index shares family with lea dst"
        );
        let ok_line = "movq 8(%r8,%rcx,8), %rax";
        let ok_pos = ok_line.find('(').unwrap();
        let ok_close = ok_line.find(')').unwrap();
        assert_eq!(
            compose_indexed_relay(ok_line, ok_pos, ok_close, "%r8", "(%rsi)").as_deref(),
            Some("8(%rsi, %rcx, 8)"),
        );
        assert_eq!(
            splice_lea_into_mem_operand(ok_line, "%r8", "(%rsi)").as_deref(),
            Some("movq 8(%rsi, %rcx, 8), %rax"),
        );
    }
}

#[cfg(test)]
mod chain_rename_tests {
    use super::super::super::peephole_optimize;

    fn run(body: &str) -> String {
        peephole_optimize(format!(".text\nf:\n.cfi_startproc\n{body}\n.cfi_endproc\n"))
    }

    /// `m3x`: `use(a) * 3 + b` — GCC 16.2 computes straight into %eax.
    #[test]
    fn producer_rmw_copy_chain_is_computed_in_the_copy_target() {
        let out = run(
            "    leal (%rax, %rax, 2), %r11d\n    addl %ebx, %r11d\n    movl %r11d, %eax\n    ret",
        );
        assert!(out.contains("leal (%rax, %rax, 2), %eax"), "{out}");
        assert!(out.contains("addl %ebx, %eax"), "{out}");
        assert!(!out.contains("%r11"), "{out}");
    }

    #[test]
    fn chain_rename_declines_unsound_windows() {
        // A later window line reads D's OLD value.
        let out = run(
            "    leal (%rcx, %rcx, 2), %r11d\n    addl %eax, %r11d\n    movl %r11d, %eax\n    ret",
        );
        assert!(out.contains("addl %eax, %r11d"), "{out}");
        // 64-bit last write under a movl copy (bits 32..63 must be dropped).
        let out = run(
            "    leaq (%rcx, %rcx, 2), %r11\n    addq %rbx, %r11\n    movl %r11d, %eax\n    ret",
        );
        assert!(!out.contains("addq %rbx, %rax"), "{out}");
        // T still read after the copy.
        let out = run(
            "    leal (%rcx, %rcx, 2), %r11d\n    addl %ebx, %r11d\n    movl %r11d, %eax\n    addl %r11d, %eax\n    ret",
        );
        assert!(out.contains("addl %ebx, %r11d"), "{out}");
        // Implicit operand in the window: `cltd` reads D's OLD %eax and
        // its %edx result is stored, so it cannot be deleted first.
        let out = run(
            "    leal (%rcx, %rcx, 2), %r11d\n    cltd\n    addl %ebx, %r11d\n    movl %r11d, %eax\n    movl %edx, (%rdi)\n    ret",
        );
        assert!(out.contains("cltd"), "{out}");
        assert!(out.contains("addl %ebx, %r11d"), "{out}");
        // Narrow T mention (a partial read the rename must not touch).
        let out = run(
            "    leal (%rcx, %rcx, 2), %r11d\n    movb %r11b, (%rdi)\n    movl %r11d, %eax\n    ret",
        );
        assert!(out.contains("%r11b"), "{out}");
    }

    /// The shapes whose `coalesce_copy_into_rmw` refusals other tests pin
    /// (with this pass skipped): the rename must leave the hazards they guard
    /// untouched — the source register is never written, flags consumers see
    /// the same flag-setting value, a live `%rdx` return half is untouched.
    #[test]
    fn chain_rename_on_coalesce_refusal_shapes_is_value_preserving() {
        let body = |out: &str| -> Vec<String> {
            out.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('.') && !l.ends_with(':'))
                .map(str::to_string)
                .collect()
        };
        let out = run("    movl %r8d, %r10d\n    andl %r8d, %r10d\n    movl %r10d, %eax\n    ret");
        assert_eq!(
            body(&out),
            ["movl %r8d, %eax", "andl %r8d, %eax", "ret"],
            "{out}"
        );
        let out = run(
            "    movl %ebx, %r11d\n    shlq $32, %r11\n    movq %r11, %rdi\n    call bar\n    ret",
        );
        assert_eq!(
            body(&out),
            ["movl %ebx, %edi", "shlq $32, %rdi", "call bar", "ret"],
            "{out}"
        );
        let out = run(
            "    movl %r8d, %r10d\n    shlq $32, %r10\n    setl %cl\n    movq %r10, %rax\n    ret",
        );
        assert_eq!(
            body(&out),
            ["movl %r8d, %eax", "shlq $32, %rax", "setl %cl", "ret"],
            "{out}"
        );
        let out = run(
            "    # LCCC_RET_RDX 1\n    movzbl %dl, %r11d\n    andl $127, %r11d\n    movl %r11d, %eax\n    ret",
        );
        assert_eq!(
            body(&out),
            [
                "# LCCC_RET_RDX 1",
                "movzbl %dl, %eax",
                "andl $127, %eax",
                "ret"
            ],
            "{out}"
        );
    }

    /// A 32-bit chain under a movq copy: the upper half is already zero.
    #[test]
    fn chain_rename_accepts_movq_copy_of_a_32bit_chain() {
        let out = run(
            "    leal (%rcx, %rcx, 4), %r10d\n    subl %esi, %r10d\n    movq %r10, %rax\n    ret",
        );
        assert!(out.contains("subl %esi, %eax"), "{out}");
        assert!(!out.contains("%r10"), "{out}");
    }

    /// `-g` must not change code: the `.loc` lines between the chain and its
    /// copy (zstd_count's `shrl`/`andl` chain had two) do not count against
    /// the window. Five per gap exhaust the old line-counting budget.
    #[test]
    fn chain_rename_window_ignores_debug_locations() {
        let plain =
            "    leal (%rax, %rax, 2), %r11d\n    addl %ebx, %r11d\n    movl %r11d, %eax\n    ret";
        let locs = ".loc 1 7 3\n".repeat(5);
        let debug = format!(
            "    leal (%rax, %rax, 2), %r11d\n{locs}    addl %ebx, %r11d\n{locs}    movl %r11d, %eax\n    ret"
        );
        let strip = |s: String| -> String {
            s.lines()
                .filter(|l| !l.trim_start().starts_with(".loc"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (a, b) = (strip(run(plain)), strip(run(&debug)));
        assert!(a.contains("addl %ebx, %eax"), "{a}");
        assert_eq!(a, b, "debug locations changed the generated code");
    }
}

#[cfg(test)]
mod scaled_index_temp_tests {
    use super::super::super::peephole_optimize;
    use super::super::super::types::{LineInfo, LineStore, classify_line};
    use super::fold_scaled_index_temp_into_mem;

    fn func(body: &[&str]) -> String {
        format!(
            ".text\nf:\n.cfi_startproc\n{}\n    ret\n.cfi_endproc\n",
            body.join("\n")
        )
    }

    /// Run ONLY this pass; returns (changed, resulting instruction lines).
    fn fold(body: &[&str]) -> (bool, Vec<String>) {
        let mut store = LineStore::new(func(body));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = fold_scaled_index_temp_into_mem(&mut store, &mut infos);
        let lines = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()) && !l.ends_with(':'))
            .collect();
        (changed, lines)
    }

    #[test]
    fn index_only_lea_folds_into_the_unscaled_index() {
        // `__builtin_prefetch(&p[i] + 16)` loop body (measured).
        let (c, out) = fold(&[
            "    leaq 0(,%r9,8), %r11",
            "    prefetcht0 128(%rdi, %r11, 1)",
        ]);
        assert!(c);
        assert_eq!(out, ["prefetcht0 128(%rdi, %r9, 8)", "ret"]);
    }

    #[test]
    fn lea_shift_pair_folds_into_a_scaled_index() {
        // `__builtin_prefetch(&d[i + 8], 1, 0)` loop body (measured).
        let (c, out) = fold(&[
            "    leaq 8(%r8), %r9",
            "    shlq $3, %r9",
            "    prefetchnta (%rdi, %r9)",
        ]);
        assert!(c);
        assert_eq!(out, ["prefetchnta 64(%rdi, %r8, 8)", "ret"]);
    }

    #[test]
    fn sole_base_temp_becomes_an_index_only_operand() {
        let (c, out) = fold(&[
            "    leaq 16(%r8), %r9",
            "    shlq $2, %r9",
            "    movl (%r9), %eax",
        ]);
        assert!(c);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(
            out[0].starts_with("movl 64(") && out[0].contains("%r8, 4)"),
            "{out:?}"
        );
    }

    #[test]
    fn producer_index_may_be_the_temporary_itself() {
        // With the producer deleted, %r9 keeps its pre-producer value.
        let (c, out) = fold(&["    leaq 0(,%r9,8), %r9", "    prefetcht0 (%rdi, %r9)"]);
        assert!(c);
        assert_eq!(out, ["prefetcht0 (%rdi, %r9, 8)", "ret"]);
    }

    #[test]
    fn declines_unsound_or_unprofitable_windows() {
        for body in [
            // %T read after the consumer.
            &[
                "    leaq 0(,%r9,8), %r11",
                "    prefetcht0 (%rdi, %r11)",
                "    movq %r11, %rax",
            ][..],
            // The deleted shl's flags are read.
            &[
                "    leaq 8(%r8), %r9",
                "    shlq $3, %r9",
                "    prefetcht0 (%rdi, %r9)",
                "    setc %al",
            ],
            // 32-bit shift: zero-extension is not lea scaling.
            &[
                "    leaq 8(%r8), %r9",
                "    shll $3, %r9d",
                "    prefetcht0 (%rdi, %r9)",
            ],
            // Scale 4 is not encodable after shifting by 4.
            &[
                "    leaq 8(%r8), %r9",
                "    shlq $4, %r9",
                "    prefetcht0 (%rdi, %r9)",
            ],
            // %T is the SCALED index: composing would need a scale product.
            &["    leaq 0(,%r9,8), %r11", "    prefetcht0 (%rdi, %r11, 2)"],
            // Two mentions of %T in the consumer.
            &["    leaq 0(,%r9,8), %r11", "    movq %r11, (%rdi, %r11)"],
            // lea consumer: base+index+disp is a slow LEA on Skylake-family.
            &["    leaq 0(,%r9,8), %r11", "    leaq 128(%rdi, %r11), %rax"],
            // Not adjacent: an intervening write to the producer's input.
            &[
                "    leaq 0(,%r9,8), %r11",
                "    addq $1, %r9",
                "    prefetcht0 (%rdi, %r11)",
            ],
            // Base equals the producer index: (%rdi)+(%rdi*8) is scale 9.
            &["    leaq 0(,%rdi,8), %r11", "    prefetcht0 (%rdi, %r11)"],
            // Segment-relative consumer.
            &[
                "    leaq 0(,%r9,8), %r11",
                "    movq %gs:(%rdi, %r11), %rax",
            ],
            // Base-and-index producer with an index consumer: three registers.
            &["    leaq (%rsi,%r9,8), %r11", "    prefetcht0 (%rdi, %r11)"],
        ] {
            let (c, _) = fold(body);
            assert!(!c, "{body:?}");
        }
    }

    /// hot_web_loop's `b ^= a + (b << 3)`: two dependent LEAs become GCC's
    /// single two-component `leaq (%r12, %r14, 8)`.
    #[test]
    fn lea_consumer_composes_to_a_two_component_address() {
        let (c, out) = fold(&[
            "    leaq 0(,%r14,8), %r11",
            "    leaq (%r12, %r11, 1), %rdi",
        ]);
        assert!(c);
        assert_eq!(out, ["leaq (%r12, %r14, 8), %rdi", "ret"]);
        // `leal`: the low 32 bits of base + index*8 are the same either way.
        let (c, out) = fold(&["    leaq 0(,%r14,8), %r11", "    leal (%r12, %r11), %edi"]);
        assert!(c);
        assert_eq!(out, ["leal (%r12, %r14, 8), %edi", "ret"]);
    }

    /// `(%T, %B, 1)`: the scale-1 temporary in base position swaps slots.
    #[test]
    fn temp_in_base_position_beside_an_unscaled_index_swaps_slots() {
        let (c, out) = fold(&["    leaq 0(,%r14,8), %r11", "    leaq (%r11, %r12), %rdi"]);
        assert!(c);
        assert_eq!(out, ["leaq (%r12, %r14, 8), %rdi", "ret"]);
        let (c, out) = fold(&["    leaq 0(,%r9,8), %r11", "    prefetcht0 16(%r11, %rsp)"]);
        assert!(c);
        assert_eq!(out, ["prefetcht0 16(%rsp, %r9, 8)", "ret"]);
    }

    /// Three-component (slow) or symbolic LEA compositions stay split.
    #[test]
    fn lea_consumer_declines_three_component_results() {
        for body in [
            &["    leaq 8(,%r9,8), %r11", "    leaq (%rdi, %r11), %rax"][..],
            &["    leaq 0(,%r9,8), %r11", "    leaq tab(%rdi, %r11), %rax"],
            &["    leaq 0(,%r9,8), %r11", "    leaq -8(%r11, %rdi), %rax"],
            &["    leaq 0(,%r9,8), %r11", "    leaq (%rdi, %r11), %r11"],
            &["    leaq 0(,%r9,8), %r11", "    leaq (%r11, %r11), %rax"],
        ] {
            let (c, _) = fold(body);
            assert!(!c, "{body:?}");
        }
    }

    /// The whole pipeline reaches GCC's shape from the measured codegen.
    #[test]
    fn pipeline_matches_gcc_for_the_software_prefetch_idiom() {
        let out = peephole_optimize(func(&[
            "    leaq 8(%r8), %r9",
            "    shlq $3, %r9",
            "    prefetchnta (%rdi, %r9)",
            "    movq (%rsi, %r8, 8), %rax",
        ]));
        assert!(out.contains("prefetchnta 64(%rdi, %r8, 8)"), "{out}");
        assert!(!out.contains("%r9"), "{out}");
    }
}

#[cfg(test)]
mod rbp_role_and_span_tests {
    use super::super::super::types::{
        LineInfo, LineStore, REG_GP_MAX, classify_line, mark_nop, replace_line,
    };
    use super::super::liveness::FileLiveness;
    use super::{LazyDeadness, RbpRoles, rbp_is_gpr_in_function};

    fn parse(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let infos = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    fn line_of(store: &LineStore, text: &str) -> usize {
        (0..store.len())
            .find(|&i| store.get(i).trim() == text)
            .unwrap_or_else(|| panic!("fixture lacks `{text}`"))
    }

    /// The verdict over a one-function program whose body is `body`.
    fn verdict(body: &str) -> bool {
        let (store, infos) = parse(&format!("f:\n.cfi_startproc\n{body}\n.cfi_endproc\n"));
        rbp_is_gpr_in_function(&store, &infos, 0, store.len() - 1)
    }

    /// One fixture per frame-pointer rule.  The `bp`-free spellings
    /// (`.cfi_def_cfa_register 6`, `leave`, `enter`) are the ones the
    /// one-substring prefilter must still let through.
    #[test]
    fn rbp_verdict_sees_every_frame_pointer_rule() {
        let gpr =
            "    pushq %rbp\n    movq %rdi, %rbp\n    addq %rbp, %rax\n    popq %rbp\n    ret";
        assert!(verdict(gpr), "a callee-saved rbp used as data is a GPR");
        for evidence in [
            "    movq %rsp, %rbp",
            "    movq %rbp, %rsp",
            "    movl %esp, %ebp",
            "    mov %ebp, %esp",
            "    leave",
            "    leaveq",
            "    enter $16, $0",
            ".cfi_def_cfa_register 6",
            ".cfi_def_cfa_register 5",
            ".cfi_def_cfa_register %rbp",
            ".cfi_def_cfa %rbp, 16",
            "    movq -8(%rbp), %rax",
            "    movl %eax, (%ebp)",
            "    leaq 16(%rbp), %rdi",
            "    movw %ax, (%bp)",
        ] {
            let body =
                format!("    pushq %rbp\n{evidence}\n    addq %rbp, %rax\n    popq %rbp\n    ret");
            assert!(
                !verdict(&body),
                "`{evidence}` marks a frame-pointer function"
            );
        }
    }

    /// A memo hit is the predicate on the current text: verdicts are kept
    /// across queries, an edit forgets exactly its function's verdict, and a
    /// line outside every function answers `false`.
    #[test]
    fn rbp_roles_memo_forgets_only_the_edited_function() {
        let (mut store, mut infos) = parse(concat!(
            "    .text\n",
            "f:\n.cfi_startproc\n",
            "    pushq %rbp\n    movq %rdi, %rbp\n    addq %rbp, %rax\n    popq %rbp\n    ret\n",
            ".cfi_endproc\n",
            "g:\n.cfi_startproc\n",
            "    pushq %rbp\n    movq %rsi, %rbp\n    addq %rbp, %rdx\n    popq %rbp\n    ret\n",
            ".cfi_endproc\n",
        ));
        let lv = FileLiveness::new(&store, &infos);
        let mut roles = RbpRoles::default();
        let in_f = line_of(&store, "addq %rbp, %rax");
        let in_g = line_of(&store, "addq %rbp, %rdx");
        assert!(
            !roles.is_gpr(&lv, &store, &infos, 0),
            "`.text` lies outside every function"
        );
        assert!(roles.is_gpr(&lv, &store, &infos, in_f));
        assert!(roles.is_gpr(&lv, &store, &infos, in_g));
        assert_eq!(roles.verdicts.len(), 2);

        // Turn f into a frame-pointer function, as a fold creating an
        // `(%rbp)` operand would.
        replace_line(
            &mut store,
            &mut infos[in_f],
            in_f,
            "    movq 8(%rbp), %rax".to_string(),
        );
        roles.edited(&lv, &store, &infos, in_f);
        assert_eq!(roles.verdicts.len(), 1, "only f's verdict is dropped");
        assert!(
            !roles.is_gpr(&lv, &store, &infos, in_f),
            "the re-scan sees the new text"
        );
        assert!(roles.is_gpr(&lv, &store, &infos, in_g));
    }

    /// Every `(line, family)` answer of `dead`'s solution equals a fresh
    /// analysis of the current text.
    fn assert_matches_fresh(dead: &LazyDeadness, store: &LineStore, infos: &[LineInfo]) {
        let lv = dead.lv.as_ref().expect("solution built");
        let fresh = FileLiveness::new(store, infos);
        for n in 0..store.len() {
            for fam in 0..=REG_GP_MAX {
                assert_eq!(
                    lv.live_after(n, fam),
                    fresh.live_after(n, fam),
                    "line {n} `{}`, family {fam}",
                    store.get(n).trim()
                );
            }
        }
    }

    const LOOP: &str = concat!(
        "f:\n.cfi_startproc\n",
        "    movq %rdi, %rax\n",
        "    xorl %esi, %esi\n",
        ".L1:\n",
        "    leaq 8(%rax), %rdx\n",
        "    movq (%rdx), %rcx\n",
        "    addq %rcx, %rsi\n",
        "    decq %r8\n",
        "    jne .L1\n",
        "    movq %rsi, %rax\n",
        "    ret\n",
        ".cfi_endproc\n",
    );

    /// `fold_lea_into_memory_op`'s edit (LEA deleted, consumer rewritten)
    /// reported as a span keeps the cached solution exact, both on the
    /// direct path and when the function is already queued for a full
    /// re-analysis by `invalidate`.
    #[test]
    fn lazy_deadness_span_refresh_equals_a_fresh_analysis() {
        for queued in [false, true] {
            let (mut store, mut infos) = parse(LOOP);
            let mut dead = LazyDeadness::new();
            let i = line_of(&store, "leaq 8(%rax), %rdx");
            let j = line_of(&store, "movq (%rdx), %rcx");
            assert!(
                dead.dead_after(&store, &infos, j, 2, &[i, j]),
                "%rdx dies at its use"
            );
            mark_nop(&mut infos[i]);
            replace_line(
                &mut store,
                &mut infos[j],
                j,
                "    movq 8(%rax), %rcx".to_string(),
            );
            if queued {
                dead.invalidate(&store, &infos, j);
            }
            dead.invalidate_span(&store, &infos, i, j);
            // Drain any queued re-analysis the way a query does.
            dead.dead_after(&store, &infos, j, 1, &[j]);
            assert_matches_fresh(&dead, &store, &infos);
        }
    }
}
