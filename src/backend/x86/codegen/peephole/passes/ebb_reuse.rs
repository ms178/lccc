//! Extended-basic-block reuse of register copies and flags.
//!
//! The block-local passes (`copy_propagation`, `fold_redundant_flags_compare`)
//! forget every fact at a label. That is the only sound default for a
//! textual sweep -- a label may join several paths -- but a label with
//! exactly ONE predecessor joins nothing: everything known at the end of the
//! predecessor still holds at its first instruction. Two such edges exist in
//! a linear sweep:
//!
//! * the fall-through into an UNMENTIONED label (no jump can reach it), and
//! * a forward `jcc .LBBn` / `jmp .LBBn` that is the ONLY mention of `.LBBn`
//!   anywhere in the file (no jump table, no address-taken use), when the
//!   line before `.LBBn:` cannot fall through into it.
//!
//! Following those edges turns the sweep into a walk over extended basic
//! blocks: the state at the branch is snapshotted for its target and resumed
//! there; every other label, and every call, `ret`, indirect jump, push/pop,
//! inline-asm region and non-inert directive, starts from nothing.
//!
//! # Facts
//!
//! * A value number per GP register family. `movq %a, %b` gives `%b` the
//!   number of `%a`; any other possible write (explicit destination,
//!   architectural implicit write, multi-output forms, or -- when the
//!   classifier knows no destination -- every register the line names) gives
//!   the written families fresh numbers. Equal numbers mean equal 64-bit
//!   contents, hence equal contents at every width.
//! * The flags as the canonical text of the last register/immediate-only
//!   `test`/`cmp` (registers spelled by value number and width), cleared by
//!   any line not proven to preserve EFLAGS (`preserves_eflags`). A later
//!   write of an operand register does not clear it: the fact describes the
//!   value the flags were computed from, and a re-test of the redefined
//!   register carries a different number, so it can never match.
//!
//! # Deletions (the only edits [`reuse_ebb_copies_and_flags`] makes)
//!
//! * `movq %a, %b` when `%b` already holds `%a`'s value.
//! * A `test`/`cmp` whose canonical form equals the live flags fact: every
//!   flag bit it would produce is already in EFLAGS.
//!
//! Motivating shape (three-way sibling dispatch,
//! `if (c > 0) return h3(x, y, c); if (c < 0) return g1(y); return g2(x);`):
//! `c` is copied to `%r11` because the `h3` arm overwrites `%rdi` before it
//! reads `c`, and the second arm's compare re-tests the copy:
//!
//! ```text
//!     movq %rdi, %r11
//!     testl %edi, %edi
//!     jle .LBB2               ; the only mention of .LBB2
//!     ...
//!     jmp h3@PLT
//! .LBB2:
//!     testl %r11d, %r11d      ; same value, flags still live: deleted
//!     jge .LBB4
//! ```
//!
//! [`sink_copies_into_fallthrough`] then moves such a copy past the `jcc`
//! when only the fall-through arm reads it, so the taken path no longer
//! executes it.
//!
//! The block-local identity fold (`fold_redundant_flags_compare`) documents
//! a measured, rejected cross-edge variant for TEXTUALLY identical compares.
//! This pass differs in what it can see (copy-equivalent operands, which the
//! register allocator's call-staging copies create in every sibling
//! dispatcher) and in cost: one linear sweep with the label census the jump
//! threading passes already compute.

use super::super::types::*;
use super::flag_peepholes::{is_byteless_comment, preserves_eflags};
use super::helpers::{get_dest_reg, parse_reg_to_reg_movq};
use super::jump_thread::{label_mention_counts, label_text, local_label};
use super::liveness::FileLiveness;
use super::relay_and_lea::split_two_operands;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Number of GP register families tracked (`REG_NAMES` columns).
const FAMILIES: usize = 16;

#[derive(Clone)]
struct State {
    vn: [u32; FAMILIES],
    flags: Option<String>,
}

impl State {
    fn fresh(next: &mut u32) -> State {
        let mut vn = [0u32; FAMILIES];
        for slot in &mut vn {
            *slot = *next;
            *next += 1;
        }
        State { vn, flags: None }
    }
}

/// `(family, width row)` of a plain GP register operand (`%r11d` →
/// `(11, 1)`); `None` for anything else, including the legacy high-byte
/// registers, which name bits a value number does not describe separately.
fn gp_operand(op: &str) -> Option<(usize, usize)> {
    (0..4).find_map(|row| {
        REG_NAMES[row]
            .iter()
            .position(|&name| name == op)
            .map(|fam| (fam, row))
    })
}

/// Canonical text of a register/immediate-only `test`/`cmp`, or `None`.
fn flags_fact(t: &str, st: &State) -> Option<String> {
    let mnemonic = t.split_ascii_whitespace().next()?;
    let (base, suffix) = mnemonic.split_at(mnemonic.len().checked_sub(1)?);
    if !matches!(base, "test" | "cmp") || !matches!(suffix, "b" | "w" | "l" | "q") {
        return None;
    }
    let (a, b) = split_two_operands(t[mnemonic.len()..].trim_start())?;
    let mut out = String::from(mnemonic);
    for op in [a, b] {
        out.push(' ');
        if let Some((fam, row)) = gp_operand(op) {
            out.push_str(&format!("v{}w{row}", st.vn[fam]));
        } else if op.starts_with('$') && !op.contains('(') {
            out.push_str(op);
        } else {
            return None;
        }
    }
    Some(out)
}

/// GP families the line may write (an over-approximation is always safe:
/// it only forgets facts).
fn written_families(info: &LineInfo, t: &str) -> u16 {
    if info.kind == LineKind::InlineAsm {
        return u16::MAX;
    }
    let mut w = implicit_write_refs(t.as_bytes());
    let mnemonic = t.split_ascii_whitespace().next().unwrap_or("");
    // Compares and plain stores name registers they only read.
    let plain_store = matches!(mnemonic, "movb" | "movw" | "movl" | "movq")
        && split_two_operands(t[mnemonic.len()..].trim_start())
            .is_some_and(|(_, dst)| dst.contains('('));
    if info.kind == LineKind::Cmp || matches!(info.kind, LineKind::StoreRbp { .. }) || plain_store {
        return w;
    }
    let dest = get_dest_reg(info);
    if dest <= REG_GP_MAX {
        w |= 1u16 << dest;
    } else {
        // No classified destination: assume every named register is written.
        w |= info.reg_refs;
    }
    if ["xchg", "xadd", "cmpxchg", "mulx"]
        .iter()
        .any(|m| mnemonic.starts_with(m))
    {
        w |= info.reg_refs;
    }
    w
}

/// The `.LBBn` target of a direct `jmp`/`jcc`, if local.
fn branch_target(t: &str) -> Option<u32> {
    let target = t.split_ascii_whitespace().nth(1)?;
    local_label(target)
}

/// Delete register copies and flag setters an extended basic block already
/// established (module docs). Returns whether anything changed.
pub(super) fn reuse_ebb_copies_and_flags(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let (mentions, duplicate) = label_mention_counts(store, infos);
    let mentioned = |l: u32| mentions.get(&l).copied().unwrap_or(0);
    let mut next_vn = 0u32;
    let mut st = State::fresh(&mut next_vn);
    let mut snapshots: FxHashMap<u32, State> = FxHashMap::default();
    let mut defined: FxHashSet<u32> = FxHashSet::default();
    // Whether the line being visited can be reached by falling through.
    let mut falls = true;
    let mut changed = false;
    for i in 0..store.len() {
        let info = infos[i];
        if info.is_nop() || info.kind == LineKind::Empty {
            continue;
        }
        let t = info.trimmed(store.get(i));
        if is_byteless_comment(t) {
            continue;
        }
        match info.kind {
            LineKind::Directive if info.inert_dir => continue,
            LineKind::Label => {
                let label = local_label(label_text(t)).filter(|l| !duplicate.contains(l));
                st = match label {
                    Some(l) if falls && mentioned(l) == 0 => st,
                    Some(l) if !falls && mentioned(l) == 1 => snapshots
                        .remove(&l)
                        .unwrap_or_else(|| State::fresh(&mut next_vn)),
                    _ => State::fresh(&mut next_vn),
                };
                if let Some(l) = label {
                    defined.insert(l);
                }
                falls = true;
                continue;
            }
            LineKind::CondJmp | LineKind::Jmp => {
                if let Some(l) = branch_target(t)
                    && mentioned(l) == 1
                    && !duplicate.contains(&l)
                    && !defined.contains(&l)
                {
                    snapshots.insert(l, st.clone());
                }
                if info.kind == LineKind::Jmp {
                    falls = false;
                    st = State::fresh(&mut next_vn);
                }
                continue;
            }
            LineKind::Ret | LineKind::JmpIndirect => {
                falls = false;
                st = State::fresh(&mut next_vn);
                continue;
            }
            LineKind::Call
            | LineKind::Push { .. }
            | LineKind::Pop { .. }
            | LineKind::InlineAsm
            | LineKind::Directive => {
                falls = true;
                st = State::fresh(&mut next_vn);
                continue;
            }
            _ => {}
        }
        if t.starts_with("ud2") {
            falls = false;
            st = State::fresh(&mut next_vn);
            continue;
        }
        falls = true;
        if let Some((src, dst)) = parse_reg_to_reg_movq(&info, t) {
            let (s, d) = (src as usize, dst as usize);
            if s < FAMILIES && d < FAMILIES {
                if st.vn[s] == st.vn[d] && !info.pinned && !info.volatile_access {
                    mark_nop(&mut infos[i]);
                    changed = true;
                } else {
                    st.vn[d] = st.vn[s];
                }
                continue;
            }
        }
        if let Some(fact) = flags_fact(t, &st) {
            if st.flags.as_deref() == Some(fact.as_str()) && !info.pinned && !info.volatile_access {
                mark_nop(&mut infos[i]);
                changed = true;
            } else {
                st.flags = Some(fact);
            }
            continue;
        }
        let w = written_families(&info, t);
        for (fam, slot) in st.vn.iter_mut().enumerate() {
            if w & (1u16 << fam) != 0 {
                *slot = next_vn;
                next_vn += 1;
            }
        }
        if !preserves_eflags(t) {
            st.flags = None;
        }
    }
    changed
}

/// Move a register copy that only the fall-through arm of the block's
/// closing `jcc` reads to just after that `jcc`.
///
/// `movq %a, %b; I1..Ik; jcc .LT` becomes `I1..Ik; jcc .LT; movq %a, %b`
/// when the `I` lines neither name `%b` nor write `%a`, `%b` is dead on entry
/// to `.LT` (CFG liveness), and nothing but the fall-through reaches the
/// line after the `jcc` (every label up to the next instruction is
/// unmentioned). The taken path then no longer executes the copy. Lines move
/// by rotating text within the block, so every line involved must be
/// editable (`all_editable`, no volatile protection).
pub(super) fn sink_copies_into_fallthrough(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let (mentions, duplicate) = label_mention_counts(store, infos);
    let mut liveness: Option<FileLiveness> = None;
    let mut changed = false;
    let len = store.len();
    let mut j = 0;
    while j < len {
        if infos[j].kind != LineKind::CondJmp {
            j += 1;
            continue;
        }
        let Some(target) = branch_target(infos[j].trimmed(store.get(j))) else {
            j += 1;
            continue;
        };
        // Only the fall-through may reach the line after the jcc.
        let mut k = j + 1;
        let mut clean_fallthrough = true;
        while k < len {
            let tk = infos[k].trimmed(store.get(k));
            if infos[k].is_nop() || infos[k].kind == LineKind::Empty || is_byteless_comment(tk) {
                k += 1;
                continue;
            }
            if infos[k].kind == LineKind::Label {
                let unmentioned = local_label(label_text(tk))
                    .is_some_and(|l| !duplicate.contains(&l) && !mentions.contains_key(&l));
                if !unmentioned {
                    clean_fallthrough = false;
                    break;
                }
                k += 1;
                continue;
            }
            break;
        }
        // The target label's line, for the liveness query.
        let target_line = (j + 1..len).find(|&n| {
            infos[n].kind == LineKind::Label
                && local_label(label_text(infos[n].trimmed(store.get(n)))) == Some(target)
        });
        let (true, Some(target_line)) = (clean_fallthrough, target_line) else {
            j += 1;
            continue;
        };
        // Walk back over the block for the latest copy whose destination the
        // lines between it and the jcc do not name.
        let mut p = j;
        let mut between: Vec<usize> = Vec::new();
        let mut candidate = None;
        while p > 0 {
            p -= 1;
            let info = infos[p];
            let tp = info.trimmed(store.get(p));
            if info.is_nop() || info.kind == LineKind::Empty || is_byteless_comment(tp) {
                continue;
            }
            if info.is_barrier() || info.pinned || info.volatile_access {
                break;
            }
            if let Some((src, dst)) = parse_reg_to_reg_movq(&info, tp) {
                let names_dst = between.iter().any(|&n| {
                    infos[n].reg_refs & (1u16 << dst) != 0
                        || implicit_read_refs(infos[n].trimmed(store.get(n)).as_bytes())
                            & (1u16 << dst)
                            != 0
                        || written_families(&infos[n], infos[n].trimmed(store.get(n)))
                            & (1u16 << dst)
                            != 0
                });
                let writes_src = between.iter().any(|&n| {
                    written_families(&infos[n], infos[n].trimmed(store.get(n))) & (1u16 << src) != 0
                });
                if !names_dst && !writes_src {
                    candidate = Some((p, dst));
                    break;
                }
            }
            between.push(p);
        }
        let Some((p, dst)) = candidate else {
            j += 1;
            continue;
        };
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        if lv.live_after(target_line, dst) != Some(false) || !lv.same_function(p, target_line) {
            j += 1;
            continue;
        }
        let span: Vec<usize> = (p..=j).collect();
        if !all_editable(infos, &span) || span.iter().any(|&n| infos[n].volatile_access) {
            j += 1;
            continue;
        }
        // Rotate the LIVE lines of the span only: a retired line keeps its
        // stale text, and `replace_line` re-classifies whatever text it is
        // given, so moving a NOP slot's text would resurrect a dead write.
        let live: Vec<usize> = span
            .iter()
            .copied()
            .filter(|&n| !infos[n].is_nop())
            .collect();
        let copy = store.get(p).to_string();
        for w in live.windows(2) {
            let next = store.get(w[1]).to_string();
            replace_line(store, &mut infos[w[0]], w[0], next);
        }
        replace_line(store, &mut infos[j], j, copy);
        changed = true;
        // Liveness is stale after the rotation; rebuild on the next query.
        liveness = None;
        j += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::types::{LineStore, classify_line};
    use super::*;

    fn run(src: &str, pass: fn(&mut LineStore, &mut [LineInfo]) -> bool) -> (bool, String) {
        let mut store = LineStore::new(src.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = pass(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        (changed, out.join("\n"))
    }

    const TAIL: &str = "    # LCCC_TAIL_CALL\n";

    fn sel3(second_test: &str) -> String {
        format!(
            "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %edi, %edi\n    jle .LBB2\n.LBB1:\n    movq %rsi, %rdi\n    movq %r11, %rdx\n    jmp h3@PLT\n{TAIL}.LBB2:\n    {second_test}\n    jge .LBB4\n.LBB3:\n    movq %rdx, %rdi\n    jmp g1@PLT\n{TAIL}.LBB4:\n    movq %rsi, %rdi\n    jmp g2@PLT\n{TAIL}.cfi_endproc\n"
        )
    }

    #[test]
    fn copy_equivalent_retest_behind_single_predecessor_label_is_deleted() {
        let (changed, out) = run(&sel3("testl %r11d, %r11d"), reuse_ebb_copies_and_flags);
        assert!(changed);
        assert_eq!(out.matches("testl").count(), 1, "{out}");
    }

    #[test]
    fn retest_of_a_redefined_copy_is_kept() {
        let src = sel3("testl %r11d, %r11d").replace(
            "    testl %edi, %edi\n    jle .LBB2",
            "    testl %edi, %edi\n    movl $5, %r11d\n    jle .LBB2",
        );
        let (changed, out) = run(&src, reuse_ebb_copies_and_flags);
        assert!(!changed, "{out}");
    }

    #[test]
    fn retest_after_a_flag_writer_is_kept() {
        let src = sel3("testl %r11d, %r11d").replace(
            "    testl %edi, %edi\n    jle .LBB2",
            "    testl %edi, %edi\n    jle .LBB2\n    addl $1, %eax",
        );
        // The add sits in the fall-through arm; the taken edge still carries
        // the original flags, so the deletion stays valid.
        let (changed, _) = run(&src, reuse_ebb_copies_and_flags);
        assert!(changed);
        let src = sel3("testl %r11d, %r11d").replace(
            "    testl %edi, %edi\n    jle .LBB2",
            "    testl %edi, %edi\n    addl $1, %eax\n    jle .LBB2",
        );
        let (changed, out) = run(&src, reuse_ebb_copies_and_flags);
        assert!(
            !changed,
            "a flag writer before the jcc re-bases the flags: {out}"
        );
    }

    #[test]
    fn label_with_a_second_mention_starts_from_nothing() {
        let src = sel3("testl %r11d, %r11d")
            .replace("    jmp g2@PLT\n", "    jmp g2@PLT\n    jmp .LBB2\n");
        let (changed, out) = run(&src, reuse_ebb_copies_and_flags);
        assert!(!changed, "{out}");
    }

    #[test]
    fn label_reachable_by_fall_through_and_jump_starts_from_nothing() {
        // Remove the tail jmp before .LBB2: the label now joins two paths.
        let src =
            sel3("testl %r11d, %r11d").replace(&format!("    jmp h3@PLT\n{TAIL}.LBB2:"), ".LBB2:");
        let (changed, out) = run(&src, reuse_ebb_copies_and_flags);
        assert!(!changed, "{out}");
    }

    #[test]
    fn backward_branch_target_is_never_resumed() {
        let src = "f:\n.cfi_startproc\n.LBB1:\n    movq %rdi, %r11\n    testl %edi, %edi\n    jle .LBB1\n    testl %r11d, %r11d\n    ret\n.cfi_endproc\n";
        // Fall-through keeps the facts (same block): the second test goes.
        let (changed, _) = run(src, reuse_ebb_copies_and_flags);
        assert!(changed);
        // But the backward target's own head is a join (entry + back edge).
        let src2 = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %edi, %edi\n.LBB1:\n    testl %r11d, %r11d\n    jle .LBB1\n    ret\n.cfi_endproc\n";
        let (changed, out) = run(src2, reuse_ebb_copies_and_flags);
        assert!(!changed, "{out}");
    }

    #[test]
    fn redundant_copy_back_is_deleted_and_memory_compares_are_ignored() {
        let src = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %edi, %edi\n    je .LBB3\n.LBB2:\n    movq %r11, %rdi\n    jmp g3@PLT\n    # LCCC_TAIL_CALL\n.LBB3:\n    cmpl $0, (%rdi)\n    cmpl $0, (%rdi)\n    ret\n.cfi_endproc\n";
        let (changed, out) = run(src, reuse_ebb_copies_and_flags);
        assert!(changed);
        assert!(!out.contains("movq %r11, %rdi"), "{out}");
        assert_eq!(out.matches("cmpl $0, (%rdi)").count(), 2, "{out}");
    }

    #[test]
    fn implicit_and_multi_output_writes_kill_value_numbers() {
        for clobber in ["cqto", "xchgq %r11, %rax", "idivq %rcx"] {
            let src = format!(
                "f:\n.cfi_startproc\n    movq %rdx, %r11\n    movq %rax, %r10\n    {clobber}\n    movq %r11, %rdx\n    movq %r10, %rax\n    ret\n.cfi_endproc\n"
            );
            let (_, out) = run(&src, reuse_ebb_copies_and_flags);
            // Every clobber writes %rdx or %r11 (cqto/idivq: %rdx; xchg: %r11).
            assert!(out.contains("movq %r11, %rdx"), "{clobber}: {out}");
            // idivq and xchg write %rax as well.
            if clobber != "cqto" {
                assert!(out.contains("movq %r10, %rax"), "{clobber}: {out}");
            }
        }
    }

    #[test]
    fn copy_used_only_by_the_fall_through_arm_sinks_past_the_jcc() {
        let src = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %edi, %edi\n    jle .LBB2\n.LBB1:\n    movq %rsi, %rdi\n    movq %r11, %rdx\n    jmp h3@PLT\n    # LCCC_TAIL_CALL\n.LBB2:\n    movq %rsi, %rdi\n    jmp g2@PLT\n    # LCCC_TAIL_CALL\n.cfi_endproc\n";
        let (changed, out) = run(src, sink_copies_into_fallthrough);
        assert!(changed, "{out}");
        let lines: Vec<&str> = out.lines().collect();
        let jle = lines.iter().position(|l| *l == "jle .LBB2").unwrap();
        assert_eq!(lines[jle + 1], "movq %rdi, %r11", "{out}");
        assert_eq!(lines[jle - 1], "testl %edi, %edi", "{out}");
    }

    #[test]
    fn rotation_leaves_retired_lines_retired_and_in_place() {
        // Reduced from the call-arg staging gate (seed 1, `f111`): an earlier
        // pass retired `movq %rdi, %r8` inside the span; rotating its stale
        // text into a live slot clobbered %r8 before the copy read it.
        let src = "f:\n.cfi_startproc\n    movq %rcx, %r10\n    movq %r8, %r11\n    movq %rdi, %r8\n    movaps %xmm0, %xmm2\n    testl %edi, %edi\n    jle .LBB2\n.LBB1:\n    movq %r11, %rdi\n    movq %r10, %rsi\n    movq %r11, %rdx\n    jmp c26@PLT\n    # LCCC_TAIL_CALL\n.LBB2:\n    movaps %xmm2, %xmm1\n    movl $24, %edi\n    jmp c6@PLT\n    # LCCC_TAIL_CALL\n.cfi_endproc\n";
        let mut store = LineStore::new(src.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let dead = (0..store.len())
            .find(|&i| store.get(i).trim() == "movq %rdi, %r8")
            .unwrap();
        mark_nop(&mut infos[dead]);
        assert!(sink_copies_into_fallthrough(&mut store, &mut infos));
        assert!(infos[dead].is_nop(), "the retired slot must stay retired");
        let live: Vec<&str> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim())
            .filter(|l| !l.is_empty())
            .collect();
        assert!(!live.contains(&"movq %rdi, %r8"), "{live:?}");
        let jle = live.iter().position(|l| *l == "jle .LBB2").unwrap();
        assert_eq!(
            live[jle - 3..jle],
            ["movq %rcx, %r10", "movaps %xmm0, %xmm2", "testl %edi, %edi"]
        );
        assert_eq!(live[jle + 1], "movq %r8, %r11", "{live:?}");
    }

    #[test]
    fn copy_live_into_the_target_or_read_before_the_jcc_stays() {
        // %r11 read in the taken arm.
        let src = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %edi, %edi\n    jle .LBB2\n.LBB1:\n    movq %r11, %rdx\n    jmp h3@PLT\n    # LCCC_TAIL_CALL\n.LBB2:\n    movq %r11, %rdi\n    jmp g2@PLT\n    # LCCC_TAIL_CALL\n.cfi_endproc\n";
        let (changed, _) = run(src, sink_copies_into_fallthrough);
        assert!(!changed);
        // %r11 read by the compare before the jcc.
        let src = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    testl %r11d, %r11d\n    jle .LBB2\n.LBB1:\n    movq %r11, %rdx\n    jmp h3@PLT\n    # LCCC_TAIL_CALL\n.LBB2:\n    xorl %edi, %edi\n    jmp g2@PLT\n    # LCCC_TAIL_CALL\n.cfi_endproc\n";
        let (changed, _) = run(src, sink_copies_into_fallthrough);
        assert!(!changed);
        // The source is redefined before the jcc.
        let src = "f:\n.cfi_startproc\n    movq %rdi, %r11\n    incl %edi\n    jle .LBB2\n.LBB1:\n    movq %r11, %rdx\n    jmp h3@PLT\n    # LCCC_TAIL_CALL\n.LBB2:\n    jmp g2@PLT\n    # LCCC_TAIL_CALL\n.cfi_endproc\n";
        let (changed, _) = run(src, sink_copies_into_fallthrough);
        assert!(!changed);
    }
}
