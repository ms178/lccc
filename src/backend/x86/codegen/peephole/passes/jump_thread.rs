//! Bypass empty direct-jump trampolines left by allocation and dead copies.
//!
//! Retarget edges, never reorder code. The only deletion is a relay the
//! retargeting orphaned: labels no line of the file mentions (so no computed
//! goto, jump table or address comparison can observe them), entered by no
//! fallthrough, holding one direct `jmp`. The skipped window
//! contains only labels, comments, deleted lines and one unconditional jump:
//! registers, memory and flags are unchanged. Keep function scope, inline asm,
//! pinned lines, semantic directives (including CFI and alignment padding),
//! short-only counter branches and cyclic/overlong chains fail-closed.
use super::super::types::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

fn local_label(s: &str) -> Option<u32> {
    let n = s.strip_prefix(".LBB")?;
    // Canonical spelling prevents distinct assembler symbols (e.g. 01 and 1)
    // from collapsing to one numeric key; oversized labels fail closed.
    if n.is_empty() || (n.len() > 1 && n.starts_with('0')) {
        return None;
    }
    n.bytes()
        .all(|c| c.is_ascii_digit())
        .then(|| n.parse().ok())
        .flatten()
}

fn label_text(s: &str) -> &str {
    s.split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches(':')
}

fn branch(s: &str) -> Option<(&str, u32)> {
    if !s.starts_with('j') {
        return None;
    }
    let mut fields = s.split('#').next()?.split_whitespace();
    let mnemonic = fields.next()?;
    let target = fields.next()?;
    // A bounded stack buffer reuses the shared recognizer without a heap
    // allocation or changing its legacy literal-space contract.
    let mut canonical = [b' '; 8];
    if mnemonic.len() >= canonical.len() {
        return None;
    }
    canonical[..mnemonic.len()].copy_from_slice(mnemonic.as_bytes());
    let opcode = std::str::from_utf8(&canonical).ok()?;
    if (mnemonic != "jmp" && !is_conditional_jump(opcode))
        || fields.next().is_some()
        || matches!(mnemonic, "jcxz" | "jecxz" | "jrcxz")
    {
        return None;
    }
    Some((mnemonic, local_label(target)?))
}

fn thread_function(store: &mut LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let mut labels = FxHashSet::default();
    for i in lo..hi {
        if infos[i].kind == LineKind::Label && !infos[i].pinned {
            let label = label_text(infos[i].trimmed(store.get(i)));
            if let Some(label) = local_label(label)
                && !labels.insert(label)
            {
                return false; // Ambiguous label: never guess which definition.
            }
        }
    }
    let mut redirects = FxHashMap::default();
    let mut next: Option<u32> = None;
    for i in (lo..hi).rev() {
        let info = infos[i];
        let s = info.trimmed(store.get(i));
        if info.pinned || info.kind == LineKind::InlineAsm {
            next = None;
            continue;
        }
        match info.kind {
            LineKind::Nop | LineKind::Empty => {}
            // Human comments emit no bytes. Keep assembler quarantine and
            // compiler-owned liveness/ABI markers opaque to this CFG rewrite.
            _ if inert_comment(s) => {}
            LineKind::Label => {
                if let Some(label) = local_label(label_text(s)).filter(|l| labels.contains(l)) {
                    if let Some(target) = next {
                        redirects.insert(label, target);
                    }
                } else {
                    next = None;
                }
            }
            LineKind::Directive if info.is_steppable() && is_debug_marker_directive(s) => {}
            _ => {
                next = branch(s).and_then(|(op, target)| {
                    (op == "jmp" && labels.contains(&target)).then_some(target)
                });
            }
        }
    }
    let mut resolved = FxHashMap::default();
    for (label, first) in &redirects {
        let mut seen = FxHashSet::default();
        seen.insert(*label);
        let mut target = *first;
        for _ in 0..64 {
            if !seen.insert(target) {
                break;
            }
            if let Some(next) = redirects.get(&target) {
                target = *next;
            } else {
                resolved.insert(*label, target);
                break;
            }
        }
    }
    let mut changed = false;
    for i in lo..hi {
        if infos[i].pinned
            // DIRECTIVE-LAW: selecting -- rewrite branches only, never directives.
            || matches!(
                infos[i].kind,
                LineKind::Nop
                    | LineKind::Empty
                    | LineKind::Directive
                    | LineKind::Label
                    | LineKind::InlineAsm
            )
        {
            continue;
        }
        let s = infos[i].trimmed(store.get(i));
        let Some((mnemonic, target)) = branch(s) else {
            continue;
        };
        let Some(final_target) = resolved.get(&target) else {
            continue;
        };
        let comment = s.find('#').map(|at| &s[at..]).unwrap_or("");
        let replacement = format!("    {mnemonic} .LBB{final_target} {comment}");
        replace_line(store, &mut infos[i], i, replacement);
        changed = true;
    }
    changed
}

/// Human comments emit no bytes; assembler quarantine and compiler-owned
/// markers stay opaque.
fn inert_comment(s: &str) -> bool {
    s.starts_with('#') && s != "#APP" && s != "#NO_APP" && !s.contains("LCCC_")
}

/// Every `.LBBn` mentioned by a live line other than as its own definition,
/// plus the labels defined more than once. Comments count: over-approximating
/// references only keeps more code.
fn label_uses(store: &LineStore, infos: &[LineInfo]) -> (FxHashSet<u32>, FxHashSet<u32>) {
    let mut used = FxHashSet::default();
    let mut defined = FxHashSet::default();
    let mut duplicate = FxHashSet::default();
    for (i, info) in infos.iter().enumerate() {
        if info.is_nop() {
            continue;
        }
        let mut s = info.trimmed(store.get(i));
        if info.kind == LineKind::Label {
            if let Some(label) = local_label(label_text(s))
                && !defined.insert(label)
            {
                duplicate.insert(label);
            }
            s = s.split_once(':').map_or("", |(_, rest)| rest);
        }
        let mut rest = s;
        while let Some(at) = rest.find(".LBB") {
            rest = &rest[at + 4..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(label) = rest[..digits].parse() {
                used.insert(label);
            }
        }
    }
    (used, duplicate)
}

/// Delete the relays [`thread_function`] orphaned: a run of unreferenced,
/// uniquely defined, unpinned labels (inert comments between them stay) that
/// follows an unconditional transfer and ends in one direct `jmp`. Nothing
/// can reach the run, so dropping it changes no execution, only dead bytes.
fn remove_dead_relays(store: &LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let mut changed = false;
    loop {
        let (used, duplicate) = label_uses(store, infos);
        let dead_label = |infos: &[LineInfo], k: usize| {
            infos[k].kind == LineKind::Label
                && !infos[k].pinned
                && local_label(label_text(infos[k].trimmed(store.get(k))))
                    .is_some_and(|l| !used.contains(&l) && !duplicate.contains(&l))
        };
        let mut found = None;
        let mut prev_transfers = false;
        let mut k = lo;
        while k < hi && found.is_none() {
            let info = infos[k];
            let s = info.trimmed(store.get(k));
            if info.is_nop() || info.kind == LineKind::Empty || inert_comment(s) {
                k += 1;
                continue;
            }
            if prev_transfers && dead_label(infos, k) {
                let mut labels = vec![k];
                let mut q = k + 1;
                while q < hi {
                    let qs = infos[q].trimmed(store.get(q));
                    if infos[q].is_nop() || infos[q].kind == LineKind::Empty || inert_comment(qs) {
                        q += 1;
                    } else if dead_label(infos, q) {
                        labels.push(q);
                        q += 1;
                    } else {
                        break;
                    }
                }
                if q < hi
                    && !infos[q].pinned
                    && infos[q].kind != LineKind::InlineAsm
                    && branch(infos[q].trimmed(store.get(q))).is_some_and(|(m, _)| m == "jmp")
                {
                    labels.push(q);
                    found = Some(labels);
                }
            }
            let mnemonic = s.split_whitespace().next().unwrap_or("");
            prev_transfers = !info.pinned
                && info.kind != LineKind::InlineAsm
                && info.kind != LineKind::Label
                && matches!(mnemonic, "jmp" | "ret" | "retq" | "ud2");
            k += 1;
        }
        let Some(lines) = found else {
            return changed;
        };
        for line in lines {
            mark_nop(&mut infos[line]);
        }
        changed = true;
    }
}

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut start = None;
    let mut changed = false;
    for i in 0..store.len() {
        // DIRECTIVE-LAW: selecting function-boundary markers, not skipping
        // directives in a motion window (only inert debug attribution is transparent).
        if infos[i].pinned || infos[i].kind != LineKind::Directive {
            continue;
        }
        let s = infos[i].trimmed(store.get(i));
        if s == ".cfi_startproc" {
            start = Some(i + 1);
        } else if s == ".cfi_endproc" {
            if let Some(lo) = start.take() {
                if thread_function(store, infos, lo, i) {
                    remove_dead_relays(store, infos, lo, i);
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(body: &str, pin: Option<&str>) -> (bool, String) {
        let mut store = LineStore::new(format!("f:\n.cfi_startproc\n{body}\n.cfi_endproc\n"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        if let Some(line) = pin {
            for (i, info) in infos.iter_mut().enumerate() {
                if store.get(i).trim() == line {
                    info.pinned = true;
                }
            }
        }
        let changed = run(&mut store, &mut infos);
        let text = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!run(&mut store, &mut infos), "must reach a fixpoint");
        (changed, text)
    }

    #[test]
    fn threads_chain_aliases_comments_and_drops_orphaned_relays() {
        for ws in [" ", "\t", "  \t "] {
            let body = format!(
                "jne{ws}.LBB1 # edge\nret\n.LBB1:\n# harmless\n.LBB2: # alias\njmp{ws}.LBB3\n.LBB3:\njmp{ws}.LBB4\n.LBB4:\nadcq $0, %rax\nret"
            );
            let (changed, out) = apply(&body, None);
            assert!(changed, "separator {ws:?}: {out}");
            assert_eq!(
                out,
                "f:\n.cfi_startproc\njne .LBB4 # edge\nret\n# harmless\n.LBB4:\nadcq $0, %rax\nret\n.cfi_endproc",
                "separator {ws:?}"
            );
        }
    }

    /// Only the orphaned, unreachable relay goes: a fallthrough entry, any
    /// mention (data, comment, `leaq`), pinning or a compiler marker keeps it.
    #[test]
    fn orphaned_relay_removal_fails_closed() {
        let (_, out) = apply(
            "jle .LBB3\nmovl $1, %eax\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            None,
        );
        assert!(out.contains("jle .LBB2") && !out.contains(".LBB3"), "{out}");
        for keep in [
            "jle .LBB3\nmovl $1, %eax\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret\n.quad .LBB3",
            "jle .LBB3\nret # .LBB3\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            "jle .LBB3\nret\n# LCCC_MARK\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret\n.LBB3:",
        ] {
            let (_, out) = apply(keep, None);
            assert!(out.contains(".LBB3:"), "{keep}: {out}");
        }
        let body = "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret";
        assert!(apply(body, Some(".LBB3:")).1.contains(".LBB3:"));
    }

    #[test]
    fn debug_markers_are_transparent_but_lookalikes_are_not() {
        for marker in [".loc 1 7 3", r#".file 2 "x.c""#, ".loc\t1 7 3"] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{marker}\njmp .LBB2\n.LBB2:\nret");
            let (changed, out) = apply(&body, None);
            assert!(changed && out.contains("jne .LBB2"), "{out}");
            assert!(out.contains(marker));
            assert!(!apply(&body, Some(marker)).0);
        }
        for marker in [
            ".location 1",
            ".file_data 2",
            ".cfi_remember_state",
            ".balign 16",
            ".quad .LBB2",
        ] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{marker}\njmp .LBB2\n.LBB2:\nret");
            assert!(!apply(&body, None).0, "{marker}");
        }
    }

    #[test]
    fn retains_address_taken_jump_table_and_fallthrough_labels() {
        let body = "leaq .LBB1(%rip), %rax\njne .LBB1\n.LBB1:\njmp .LBB2\n.LBB2:\nret\n.quad .LBB1\n.long .LBB1-.LBB2";
        let (_, out) = apply(body, None);
        for text in [
            "leaq .LBB1(%rip)",
            ".quad .LBB1",
            ".long .LBB1-.LBB2",
            ".LBB1:",
            "jmp .LBB2",
        ] {
            assert!(out.contains(text), "{text}: {out}");
        }
        for label in [".LBB01", ".LBB4294967296", ".LBB", ".LBB-1"] {
            assert!(local_label(label).is_none());
        }
    }

    #[test]
    fn refuses_effects_directives_and_pinned_trampolines() {
        for barrier in [
            "addq $1, %rax",
            "movq %rax, (%rdi)",
            "testl %eax, %eax",
            "call other",
            "endbr64",
            ".cfi_def_cfa_offset 16",
            ".p2align 4,0xcc",
            ".byte 0xcc",
            ".section .data",
            "#APP\nmovq %rax, %rbx\n#NO_APP",
            "#APP\njmp .LBB2\n#NO_APP",
        ] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{barrier}\njmp .LBB2\n.LBB2:\nret");
            assert!(!apply(&body, None).0, "crossed {barrier}");
        }
        let body = "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret";
        assert!(!apply(body, Some("jmp .LBB2")).0);
        let (_, out) = apply(body, Some("jne .LBB1"));
        assert!(out.contains("jne .LBB1"));
    }

    #[test]
    fn refuses_cycles_unknown_targets_indirect_and_short_only_branches() {
        for body in [
            "jne .LBB1\nret\n.LBB1:\njmp .LBB1",
            "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\njmp .LBB1",
            "jne .LBB1\nret\n.LBB1:\njmp .LBB99",
            "jne .LBB1\nret\n.LBB1:\njmp *%rax",
            "jrcxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
            "jecxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
            "jcxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
        ] {
            assert!(!apply(body, None).0, "{body}");
        }
    }

    #[test]
    fn refuses_overlong_chains_cross_function_edges_and_duplicate_labels() {
        let mut body = "jne .LBB0\nret\n".to_owned();
        for i in 0..66 {
            body += &format!(".LBB{i}:\njmp .LBB{}\n", i + 1);
        }
        body += ".LBB66:\nret";
        // Tail portions may shorten, but the over-budget entry is unchanged.
        let mut store = LineStore::new(format!(".cfi_startproc\n{body}\n.cfi_endproc"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        assert_eq!(store.get(1), "jne .LBB0");
        assert!(
            !apply(
                "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.cfi_endproc\ng:\n.cfi_startproc\n.LBB2:\nret",
                None
            )
            .0
        );
        assert!(!apply("jne .LBB1\n.LBB1:\njmp .LBB2\n.LBB1:\n.LBB2:\nret", None).0);
    }
}
