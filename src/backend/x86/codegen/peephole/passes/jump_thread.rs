//! Bypass empty direct-jump trampolines left by allocation and dead copies.
//!
//! Retarget edges, never remove/reorder code or labels. The skipped window
//! contains only labels, comments, deleted lines and one unconditional jump:
//! registers, memory and flags are unchanged. Keep function scope, inline asm,
//! pinned lines, every directive (including CFI and custom alignment padding),
//! short-only counter branches and cyclic/overlong chains fail-closed.
use super::super::types::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

fn local_label(s: &str) -> bool {
    s.strip_prefix(".LBB")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

fn label_text(s: &str) -> &str {
    s.split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches(':')
}

fn branch(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('j') {
        return None;
    }
    let mut fields = s.split('#').next()?.split_whitespace();
    let mnemonic = fields.next()?;
    let target = fields.next()?;
    // Reuse the shared opcode recognizer with a canonical separator;
    // LineKind's legacy fixed-space fast path does not classify jmp-tab.
    if (mnemonic != "jmp" && !is_conditional_jump(&format!("{mnemonic} ")))
        || fields.next().is_some()
        || !local_label(target)
        || matches!(mnemonic, "jcxz" | "jecxz" | "jrcxz")
    {
        return None;
    }
    Some((mnemonic, target))
}

fn thread_function(store: &mut LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let mut labels = FxHashSet::default();
    for i in lo..hi {
        if infos[i].kind == LineKind::Label && !infos[i].pinned {
            let label = label_text(infos[i].trimmed(store.get(i)));
            if local_label(label) && !labels.insert(label.to_owned()) {
                return false; // Ambiguous label: never guess which definition.
            }
        }
    }
    let mut redirects = FxHashMap::default();
    let mut next: Option<String> = None;
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
            _ if s.starts_with('#') && s != "#APP" && s != "#NO_APP" && !s.contains("LCCC_") => {}
            LineKind::Label => {
                let label = label_text(s);
                if labels.contains(label) {
                    if let Some(target) = &next {
                        redirects.insert(label.to_owned(), target.clone());
                    }
                } else {
                    next = None;
                }
            }
            _ if branch(s).is_some_and(|(mnemonic, _)| mnemonic == "jmp") => {
                next = branch(s)
                    .and_then(|(_, target)| labels.contains(target).then(|| target.to_owned()));
            }
            _ => next = None,
        }
    }
    let mut resolved = FxHashMap::default();
    for (label, first) in &redirects {
        let mut seen = FxHashSet::default();
        seen.insert(label.as_str());
        let mut target = first.as_str();
        for _ in 0..64 {
            if !seen.insert(target) {
                break;
            }
            if let Some(next) = redirects.get(target) {
                target = next;
            } else {
                resolved.insert(label.clone(), target.to_owned());
                break;
            }
        }
    }
    let mut changed = false;
    for i in lo..hi {
        if infos[i].pinned
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
        let Some(final_target) = resolved.get(target) else {
            continue;
        };
        let comment = s.find('#').map(|at| &s[at..]).unwrap_or("");
        let replacement = format!("    {mnemonic} {final_target} {comment}");
        replace_line(store, &mut infos[i], i, replacement);
        changed = true;
    }
    changed
}

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut start = None;
    let mut changed = false;
    for i in 0..store.len() {
        // DIRECTIVE-LAW: selecting function-boundary markers, not skipping
        // directives in a motion window (all window directives are barriers).
        if infos[i].pinned || infos[i].kind != LineKind::Directive {
            continue;
        }
        let s = infos[i].trimmed(store.get(i));
        if s == ".cfi_startproc" {
            start = Some(i + 1);
        } else if s == ".cfi_endproc" {
            if let Some(lo) = start.take() {
                changed |= thread_function(store, infos, lo, i);
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
            .map(|i| store.get(i))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!run(&mut store, &mut infos), "must reach a fixpoint");
        (changed, text)
    }

    #[test]
    fn threads_chain_aliases_comments_and_keeps_every_label() {
        for ws in [" ", "\t", "  \t "] {
            let body = format!(
                "jne{ws}.LBB1 # edge\nret\n.LBB1:\n# harmless\n.LBB2: # alias\njmp{ws}.LBB3\n.LBB3:\njmp{ws}.LBB4\n.LBB4:\nadcq $0, %rax\nret"
            );
            let (changed, out) = apply(&body, None);
            assert!(changed, "separator {ws:?}: {out}");
            assert!(out.contains("jne .LBB4 # edge"), "{out}");
            for label in 1..=4 {
                assert!(out.contains(&format!(".LBB{label}:")));
            }
            assert!(out.contains("adcq $0, %rax")); // Incoming flags untouched.
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
