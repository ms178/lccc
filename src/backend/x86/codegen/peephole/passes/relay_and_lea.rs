//! Generic x86-64 codegen peepholes: register move-relay elimination and
//! windowed `lea`→memory-operand folding.
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
use super::helpers::{get_dest_reg, has_implicit_reg_usage, implicit_read_reg_family, writes_family};
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
    !line_refs_family(src_part, fam)
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
                return false
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
///   is width-exact (`movl %eax,%r10d` never feeds `addq %r10,...`).
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
        let src_text = src_text.to_string();
        let dst_text = dst_text.to_string();
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
                            if st_src == dst_text && st_dst.contains('(') {
                                let new_line = format!("    {}{}, {}", sop, src_text, st_dst);
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
                            if use_src == dst_text && dst_is_other_reg {
                                let new_line = format!("    {}{}, {}", op, src_text, use_dst);
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
        if disp.contains(',') || disp.contains('(') || disp.contains(' ')
            || !disp.chars().all(|c| c.is_ascii_alphanumeric() || ".$_+-@".contains(c))
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
            rewrites.push((k, t, new_t));
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
    let ren = |op: &str| -> Option<String> {
        if let Some(f) = plain_gp_operand(op) {
            if f != from {
                return Some(op.to_string());
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
}
