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
    get_dest_reg, has_implicit_reg_usage, implicit_read_reg_family, writes_family,
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
    fam <= REG_GP_MAX && fam != 4 && fam != 5
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

/// Registers an ABI-visible control transfer can READ without naming them in
/// the instruction text: the SysV argument registers (`%rdi %rsi %rdx %rcx
/// %r8 %r9`), `%rax` (vector-argument count for variadic callees) and `%r10`
/// (the static chain for nested functions — see
/// `backend/x86/codegen/nested_fn.rs`). Whole-function textual uniqueness says
/// nothing about those reads, so a function containing a call, a tail jump or
/// an indirect jump cannot use proof 2 for these families.
#[inline]
fn implicit_at_transfer(fam: RegId) -> bool {
    matches!(fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10)
}

/// Registers `ret` reads implicitly: the integer return value `%rax:%rdx`.
#[inline]
fn implicit_at_return(fam: RegId) -> bool {
    matches!(fam, 0 | 2)
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
    let mask = 1u16 << fam;
    let Some((start, end)) = function_range(store, infos, idx) else {
        return false;
    };
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
        // ABI-implicit reads at control transfers are invisible to the text
        // scan: `call foo` reads %rdi..%r9/%rax/%r10, `ret` reads %rax:%rdx.
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
    provably_dead(store, infos, use_idx, fam, owned)
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
/// * `%S` and `%D` are distinct general-purpose registers (not `%rsp`/`%rbp`),
///   and the copy is a plain register-to-register `movl`/`movq`.
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
        if src_fam == dst_fam || !is_relayable_family(src_fam) || !is_relayable_family(dst_fam) {
            i += 1;
            continue;
        }
        let copy_w = dest_width(dst_text).unwrap_or(0);
        let src_mask = 1u16 << src_fam;
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
                                    lv.refresh_at(store, infos, j);
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
                                    lv.refresh_at(store, infos, j);
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
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let lea = infos[i].trimmed(store.get(i)).to_string();
        let Some((addr_text, dst_text, addr_fams)) = parse_lea_address(&lea) else {
            i += 1;
            continue;
        };
        let dst_fam = register_family_fast(dst_text);
        if !is_relayable_family(dst_fam) || addr_fams.contains(&dst_fam) {
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
            if infos[j].is_nop() {
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
            // irreproducible at the use site.
            if infos[j].reg_refs & addr_mask != 0 {
                let w = get_dest_reg(&infos[j]);
                if w != REG_NONE && addr_fams.contains(&w) {
                    break;
                }
            }
            if infos[j].reg_refs & dst_mask != 0 {
                // Only a BARE `(%T)` operand or a base/offset operand
                // `(%T, …)` can absorb the LEA. `8(%T)` must not match:
                // splicing would produce `8DISP(%base)`.
                let mut matched: Option<(usize, usize, String)> = None;
                if let Some((pos, _)) = t.match_indices(&addr_pat).find(|(pos, _)| {
                    *pos == 0 || matches!(t.as_bytes()[pos - 1] as char, ' ' | ',' | '\t')
                }) {
                    matched = Some((pos, pos + addr_pat.len() - 1, folded_addr.clone()));
                } else if let Some((pos, _)) = t.match_indices(&idx_pat).find(|(pos, _)| {
                    *pos == 0 || matches!(t.as_bytes()[pos - 1] as char, ' ' | ',' | '\t')
                }) {
                    if let Some(cl) = t[pos..].find(')').map(|c| pos + c) {
                        let inner = &t[pos + 1..cl];
                        if inner.starts_with(dst_text) {
                            let tail = &inner[dst_text.len()..];
                            // The spliced operand may hold at most TWO register
                            // slots (base + index). The leaq's own address may
                            // already consume both (e.g. `leaq (%rcx,%r9)`):
                            // folding that into an indexed use would emit an
                            // invalid three-register SIB
                            // (`(%rcx, %r9, %r11, 8)` — vectorize_matmul_tail).
                            let mut extra_regs = 0usize;
                            let mut fields_ok = true;
                            for (n, f) in tail.split(',').skip(1).enumerate() {
                                let f = f.trim();
                                if f.starts_with('%') {
                                    extra_regs += 1;
                                } else if n == 1 && matches!(f, "1" | "2" | "4" | "8") {
                                    // scale (only meaningful behind an index)
                                } else {
                                    fields_ok = false;
                                }
                            }
                            // x86-64 RIP-relative addressing (`disp(%rip)`)
                            // cannot carry an index register at all — the
                            // encoding is ModRM mod=00 r/m=101, no SIB. An
                            // assembler may silently DROP the index instead of
                            // erroring (observed: `leaq V(%rip,%r11), %r10`
                            // encoded as plain `leaq V(%rip), %r10` —
                            // wide_cond_zero_test wrong-code). So a %rip
                            // leaq may only fold into a BARE `(%T)` use.
                            if fields_ok
                                && !folded_addr.contains("%rip")
                                && addr_fams.len() + extra_regs <= 2
                            {
                                if let Some(aopen) = folded_addr.find('(') {
                                    let new_operand = format!(
                                        "{}{}{})",
                                        &folded_addr[..aopen + 1],
                                        &folded_addr[aopen + 1..folded_addr.len() - 1],
                                        tail
                                    );
                                    matched = Some((pos, cl, new_operand));
                                }
                            }
                        }
                    }
                }
                if let Some((op, cl, new_operand)) = matched {
                    let replacement = format!("{}{}{}", &t[..op], new_operand, &t[cl + 1..]);
                    if replacement != t
                        && !line_refs_family(&replacement, dst_fam)
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
///   mention the copy's destination family anywhere;
/// * the copy is an adjacent plain register move whose width is compatible —
///   a 32-bit producer may feed a `movl` or a `movq` copy (both leave the
///   destination zero-extended), a 64-bit producer only a `movq` copy
///   (retargeting a 64-bit producer under a `movl` copy would keep bits 32..63
///   that the copy discarded);
/// * the producer's register is provably dead after the copy (module header).
pub(super) fn retarget_producer_into_copy(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
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
        if !is_relayable_family(a_fam) {
            i += 1;
            continue;
        }
        // Next real instruction must be the copy.
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
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
        // The copy must read exactly the register the producer wrote. The
        // NAMES may differ in width (`movzbl …, %eax` followed by
        // `movq %rax, %r10`): a 32-bit write zero-extends, so the 64-bit read
        // is the same value. A 64-bit producer under a narrow copy is rejected
        // by the width rule below.
        if register_family_fast(copy_src) != a_fam || plain_gp_operand(copy_src).is_none() {
            i += 1;
            continue;
        }
        let Some(d_fam) = plain_gp_operand(copy_dst) else {
            i += 1;
            continue;
        };
        if d_fam == a_fam || !is_relayable_family(d_fam) {
            i += 1;
            continue;
        }
        // A 64-bit producer under a 32-bit copy would keep the upper half.
        if prod_w == 64 && copy_w == 32 {
            i += 1;
            continue;
        }
        // The producer must not read the destination family (its source is
        // evaluated before the write, but a rewrite would alias them).
        if line_refs_family(prod_src, d_fam) || line_refs_family(copy_dst, a_fam) {
            i += 1;
            continue;
        }
        if !provably_dead_lv(&lv, store, infos, j, a_fam, &[i, j]) {
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
        if line_refs_family(&new_line, a_fam) {
            i += 1;
            continue;
        }
        replace_line(store, &mut infos[i], i, new_line);
        mark_nop(&mut infos[j]);
        lv.refresh_at(store, infos, i);
        changed = true;
        i = j + 1;
    }
    changed
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
/// Only copies whose destination family is referenced by the LEA's source
/// are handled here; the unaliased producer+copy case belongs to
/// `retarget_producer_into_copy`.
pub(super) fn fold_copy_into_lea_base(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
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
        // The adjacent (NOPs apart) copy of the LEA result.
        let mut j = li + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
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
            lv = lv2;
            changed = true;
            i = j + 1;
        } else {
            replace_line(store, &mut infos[li], li, orig_lea);
            replace_line(store, &mut infos[j], j, orig_copy);
            for (idx, orig, _) in rewrites {
                replace_line(store, &mut infos[idx], idx, orig);
            }
            // `lv` still describes the restored text.
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
///    compensated by `shift_cf_safe_after`), and ANY 64-bit consumer folds
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
/// computes identical VALUES but divergent CF (0 vs the source's bit 32),
/// so the fold stands only when the consumer's CF is provably unobservable
/// (see `shift_cf_safe_after`). OF/AF-after-shift are undefined in BOTH
/// spellings (LLVM models them as `undef` — reading them is garbage either
/// way); only CF (defined-but-divergent) is audited. Comparisons are
/// case-insensitive: the consumer matcher is case-sensitive (missed folds
/// are safe), but a missed CF READER would be unsound.
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
    "adc", "adcb", "adcw", "adcl", "adcq", "adcx", "sbb", "sbbb", "sbbw", "sbbl", "sbbq", "rcl",
    "rclb", "rclw", "rcll", "rclq", "rcr", "rcrb", "rcrw", "rcrl", "rcrq", "salc", "cmc", "aaa",
    "aas", "aam", "aad", "daa", "das", "setb", "setc", "setnae", "setnb", "setnc", "setae",
    "cmovb", "cmovc", "cmovnae", "cmovnb", "cmovnc", "cmovae", "jb", "jc", "jnae", "jnb", "jnc",
    "jae", "lahf", "pushf", "pushfw", "pushfd", "pushfq", "syscall", "sysenter", "int", "int1",
    "int3", "into",
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
    "rdseed",
    "andn",
    "bextr",
    "blsi",
    "blsmsk",
    "blsr",
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
    "fcomi",
    "fcomip",
    "fucomi",
    "fucomip",
    "iret",
    "iretd",
    "iretq",
    "sysret",
    "sysexit",
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
    "pdep",
    "pext",
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
fn shift_cf_safe_after(store: &LineStore, infos: &[LineInfo], j: usize, len: usize) -> bool {
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
        if CF_READERS.iter().any(|lit| tok.eq_ignore_ascii_case(lit)) {
            return false;
        }
        if CF_CLOBBER.iter().any(|lit| tok.eq_ignore_ascii_case(lit)) {
            return true;
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
        if SHIFTS_GUARDED
            .iter()
            .any(|lit| tok.eq_ignore_ascii_case(lit))
        {
            if shift_proves_cf_clobber(tok, t) {
                return true;
            }
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
fn upper32_zero_at(store: &LineStore, infos: &[LineInfo], li: usize, s_fam: RegId) -> bool {
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
        // needs `shift_cf_safe_after`. The count check masks to 6 bits:
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
                && shift_cf_safe_after(store, infos, j, len))
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
        // so the source's OLD value must be dead at the copy.
        if !provably_dead_lv(&lv, store, infos, li, s_fam, &[li]) {
            continue;
        }
        // --- rename window: plain-register reads of %D until the first full
        // --- redefinition of %D or a barrier. Collect rewrites; abort on
        // --- anything unrenamable.
        let d_mask = 1u16 << d_fam;
        let mut rewrites: Vec<(usize, String, String)> = vec![(j, orig_cons, new_cons)];
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
        let lv2 = FileLiveness::new(store, infos);
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
            // `lv` still describes the restored text.
        }
    }
    changed
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

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    #[test]
    fn relay_copy_is_folded_into_alu_source() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rsi,%rbx), %eax\n",
            "    movl %eax, %r10d\n",
            "    addl %r10d, %r8d\n",
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
    fn relay_is_kept_across_a_call() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r10d\n",
            "    call bar\n",
            "    addl %r10d, %r8d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %eax, %r10d"), "{out}");
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
        // defeats whole-function `copy_coalesce`.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r12, %r11\n",
            "    andl $2080895, %r11d\n",
            "    movl %r11d, %eax\n",
            "    call bar\n",
            "    movq $5, %r12\n",
            "    addq %r12, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $2080895, %r12d"), "{out}");
        assert!(out.contains("movl %r12d, %eax"), "{out}");
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
        // (The `call` observes the stack and `%rbx` is live below, so no
        // other pass deletes the pair; the audit runs in the width gate,
        // before either deadness proof, so the refusal isolates the veto.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r8d, %r10d\n",
            "    shlq $32, %r10\n",
            "    pushq %rbx\n",
            "    call qux\n",
            "    popq %rbx\n",
            "    movl %ebx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("pushq %rbx"), "{out}");
        assert!(out.contains("movl %r8d, %r10d"), "{out}");
        assert!(out.contains("shlq $32, %r10"), "{out}");
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
        // pair folds. The source is callee-saved (transparent to the call:
        // preserved and unread after, hence dead); an argument-register or
        // static-chain (`%r10`) source would stay live into the call and
        // refuse on the value proof instead — see the next test.
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
        assert!(!out.contains("movl %ebx"), "{out}");
        assert!(out.contains("shlq $32, %rbx"), "{out}");
        assert!(out.contains("movq %rbx, %rdi"), "{out}");
        assert!(out.contains("call bar"), "{out}");
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
            "    movl $5, %eax\n",
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
        // %r10 carries the static chain of a nested-function call.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rbx, %r10\n",
            "    addq %r10, %rsi\n",
            "    call nested.0\n",
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

    #[test]
    fn displacement_form_use_is_not_folded() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    movzbl 8(%r10), %r14d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 1(%rbx), %r10"), "{out}");
        assert!(out.contains("movzbl 8(%r10), %r14d"), "{out}");
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
}
