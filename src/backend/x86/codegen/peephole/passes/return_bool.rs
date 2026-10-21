//! Dependency-free terminal integer boolean materialization.
//!
//! `cmp/test; setcc tmp8; movzx tmp8,eax; ret` needlessly merges a partial
//! temporary and then copies it. Clear eax BEFORE the flag producer and set al
//! directly. This removes the extension and often two REX prefixes without
//! changing function alignment. The comparison must not read any rax alias.
use super::super::types::*;

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for a in 0..infos.len() {
        if infos[a].is_nop() || infos[a].pinned || infos[a].reg_refs & 1 != 0 {
            continue;
        }
        let cmp = infos[a].trimmed(store.get(a));
        let Some((op, args)) = cmp.split_once(char::is_whitespace) else {
            continue;
        };
        if !matches!(
            op,
            "cmpb" | "cmpw" | "cmpl" | "cmpq" | "testb" | "testw" | "testl" | "testq"
        ) || args.contains(['(', ')', '\n', '#'])
        {
            continue;
        }
        let b = next_non_debug(store, infos, a + 1, infos.len());
        let c = next_non_debug(store, infos, b + 1, infos.len());
        let d = next_non_debug(store, infos, c + 1, infos.len());
        if d >= infos.len() || !all_editable(infos, &[a, b, c, d]) {
            continue;
        }
        let set = infos[b].trimmed(store.get(b));
        let Some(cc) = parse_setcc(set) else { continue };
        let Some((_, byte)) = set.split_once(char::is_whitespace) else {
            continue;
        };
        let byte = byte.trim();
        let Some(family) = REG_NAMES[3].iter().position(|&r| r == byte) else {
            continue;
        };
        // Do not change an ABI-preserved register's observable low byte.
        if !matches!(family, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11) {
            continue;
        }
        let ext = infos[c].trimmed(store.get(c));
        let Some(rest) = ext
            .strip_prefix("movzbl ")
            .or_else(|| ext.strip_prefix("movzbq "))
        else {
            continue;
        };
        let Some((src, dst)) = rest.split_once(',') else {
            continue;
        };
        if src.trim() != byte || !matches!(dst.trim(), "%eax" | "%rax") {
            continue;
        }
        // Enforce the destination spelling appropriate to the extension.
        if (ext.starts_with("movzbl ") && dst.trim() != "%eax")
            || (ext.starts_with("movzbq ") && dst.trim() != "%rax")
        {
            continue;
        }
        if !matches!(infos[d].trimmed(store.get(d)), "ret" | "retq") {
            continue;
        }
        let cmp = format!("    {cmp}");
        let set = format!("    set{cc} %al");
        replace_line(store, &mut infos[a], a, "    xorl %eax, %eax".into());
        replace_line(store, &mut infos[b], b, cmp);
        replace_line(store, &mut infos[c], c, set);
        changed = true;
    }
    fold_negated_copy(store, infos) | fold_return_select(store, infos) | changed
}

/// Coalesce the short-lived negated arm with its result home. All work
/// stays at the same program points, and the optional flag producer cannot
/// observe either the temporary or the earlier-written result register.
fn fold_negated_copy(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    use super::liveness::FileLiveness;
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    for a in 0..infos.len() {
        if infos[a].is_nop() || infos[a].pinned {
            continue;
        }
        let t = infos[a].trimmed(store.get(a));
        let (suffix, width, ops) = if let Some(ops) = t.strip_prefix("movl ") {
            ("l", 1usize, ops)
        } else if let Some(ops) = t.strip_prefix("movq ") {
            ("q", 0usize, ops)
        } else {
            continue;
        };
        let Some((s, d)) = ops.split_once(',') else {
            continue;
        };
        let (s, d) = (s.trim(), d.trim());
        if !REG_NAMES[width].contains(&s) {
            continue;
        }
        let Some(temp) = REG_NAMES[width].iter().position(|r| *r == d) else {
            continue;
        };
        if !matches!(temp, 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11) {
            continue;
        }
        let b = next_non_debug(store, infos, a + 1, infos.len());
        if b >= infos.len() || infos[b].trimmed(store.get(b)) != format!("neg{suffix} {d}") {
            continue;
        }
        let mut c = next_non_debug(store, infos, b + 1, infos.len());
        if c < infos.len() {
            let mid = infos[c].trimmed(store.get(c));
            let op = mid.split_whitespace().next().unwrap_or("");
            if matches!(
                op,
                "testb" | "testw" | "testl" | "testq" | "cmpb" | "cmpw" | "cmpl" | "cmpq"
            ) && infos[c].reg_refs & ((1 << temp) | 1) == 0
                && !infos[c].pinned
            {
                c = next_non_debug(store, infos, c + 1, infos.len());
            }
        }
        if c >= infos.len()
            || !all_editable(infos, &[a, b, c])
            || infos[c].trimmed(store.get(c)) != format!("mov{suffix} {d}, {}", REG_NAMES[width][0])
            || lv.live_after(c, temp as RegId) != Some(false)
        {
            continue;
        }
        let init = format!("    mov{suffix} {s}, {}", REG_NAMES[width][0]);
        replace_line(store, &mut infos[a], a, init);
        replace_line(
            store,
            &mut infos[b],
            b,
            format!("    neg{suffix} {}", REG_NAMES[width][0]),
        );
        mark_nop(&mut infos[c]);
        lv.refresh_span(store, infos, a, c);
        changed = true;
    }
    changed
}

/// Commute a terminal register select into the ABI result register. This
/// removes the post-CMOV dependency (not an instruction-count win):
/// `cmovcc S,D; mov D,A; ret` -> `mov S,A; cmov!cc D,A; ret`.
/// Keep identical 32/64-bit widths and reject memory sources. No shift flag
/// assumptions are made: a preceding TEST, especially after BMI2 or an
/// effectively-zero variable shift, remains intact.
fn fold_return_select(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    use super::liveness::FileLiveness;
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    for a in 0..infos.len() {
        if infos[a].is_nop() || infos[a].pinned {
            continue;
        }
        let t = infos[a].trimmed(store.get(a));
        let Some((op, operands)) = t.split_once(' ') else {
            continue;
        };
        let Some(cc_width) = op.strip_prefix("cmov") else {
            continue;
        };
        let (cc, width) = match cc_width.strip_suffix('l') {
            Some(cc) => (cc, 1usize),
            None => match cc_width.strip_suffix('q') {
                Some(cc) => (cc, 0usize),
                None => continue,
            },
        };
        let inverse = invert_cc(cc);
        if inverse == cc {
            continue;
        }
        let Some((src, dst)) = operands.split_once(',') else {
            continue;
        };
        let (src, dst) = (src.trim(), dst.trim());
        let Some(sfam) = REG_NAMES[width].iter().position(|r| *r == src) else {
            continue;
        };
        let Some(dfam) = REG_NAMES[width].iter().position(|r| *r == dst) else {
            continue;
        };
        // Never change a preserved register or a second ABI return value.
        if !matches!(dfam, 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11) {
            continue;
        }
        let b = next_non_debug(store, infos, a + 1, infos.len());
        let c = next_non_debug(store, infos, b + 1, infos.len());
        if c >= infos.len() || !all_editable(infos, &[a, b, c]) {
            continue;
        }
        let suffix = if width == 0 { "q" } else { "l" };
        let copy = format!("mov{suffix} {dst}, {}", REG_NAMES[width][0]);
        if infos[b].trimmed(store.get(b)) != copy
            || !matches!(infos[c].trimmed(store.get(c)), "ret" | "retq")
            || lv.live_after(b, dfam as RegId) != Some(false)
        {
            continue;
        }
        let init = format!(
            "    mov{suffix} {}, {}",
            REG_NAMES[width][sfam], REG_NAMES[width][0]
        );
        let select = format!(
            "    cmov{inverse}{suffix} {}, {}",
            REG_NAMES[width][dfam], REG_NAMES[width][0]
        );
        replace_line(store, &mut infos[a], a, init);
        replace_line(store, &mut infos[b], b, select);
        lv.refresh_span(store, infos, a, b);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn apply(body: &str) -> (bool, String) {
        let mut store = LineStore::new(body.into());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        let changed = run(&mut store, &mut infos);
        (changed, store.build_result(|i| infos[i].is_nop()))
    }
    #[test]
    fn all_conditions_widths_and_caller_saved_bytes() {
        for op in ["testl %edi, %edi", "cmpq %r9, %r10", "cmpb $3, %dil"] {
            for cc in [
                "e", "ne", "l", "le", "g", "ge", "b", "be", "a", "ae", "o", "no", "s", "ns", "p",
                "np",
            ] {
                for byte in [
                    "%al", "%cl", "%dl", "%sil", "%dil", "%r8b", "%r9b", "%r10b", "%r11b",
                ] {
                    let (changed, out) =
                        apply(&format!("{op}\nset{cc} {byte}\nmovzbl {byte}, %eax\nret\n"));
                    assert!(changed, "{op} {cc} {byte}");
                    assert!(out.contains("xorl %eax, %eax"));
                    assert!(out.contains(&format!("set{cc} %al")));
                    assert!(!out.contains("movzb"));
                    assert!(!apply(&out).0, "not idempotent");
                }
            }
        }
    }
    #[test]
    fn blocks_aliases_barriers_nonterminal_and_preserved_registers() {
        for input in [
            "testl %eax, %edi\nsete %sil\nmovzbl %sil, %eax\nret\n",
            "cmpq %rax, %rdi\nsete %sil\nmovzbl %sil, %eax\nret\n",
            "testl (%rdi), %esi\nsete %sil\nmovzbl %sil, %eax\nret\n",
            "testl %edi, %edi\nL:\nsete %sil\nmovzbl %sil, %eax\nret\n",
            "testl %edi, %edi\nsete %sil\nmovzbl %sil, %eax\naddl $1, %eax\nret\n",
            "testl %edi, %edi\nsete %bl\nmovzbl %bl, %eax\nret\n",
            "testl %edi, %edi\nsete %ah\nmovzbl %ah, %eax\nret\n",
            "#APP\ntestl %edi, %edi\nsete %sil\nmovzbl %sil, %eax\nret\n#NO_APP\n",
            "testl %edi, %edi\nsete %sil\nmovzbl %sil, %edx\nret\n",
        ] {
            assert!(!apply(input).0, "accepted {input}");
        }
        let mut store =
            LineStore::new("testl %edi, %edi\nsete %sil\nmovzbl %sil, %eax\nret\n".into());
        let initial: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        for pinned in 0..4 {
            let mut infos = initial.clone();
            infos[pinned].pinned = true;
            assert!(!run(&mut store, &mut infos));
        }
    }
    #[test]
    fn survives_whole_pipeline() {
        let input = ".text\nf:\n.cfi_startproc\ntestl %edi, %edi\nsete %sil\nmovzbl %sil, %eax\nret\n.cfi_endproc\n";
        let out = super::super::peephole_optimize(input.into());
        assert!(out.contains("xorl %eax, %eax"), "{out}");
        assert!(out.contains("sete %al"), "{out}");
        assert!(!out.contains("movzbl"), "{out}");
    }
}

#[cfg(test)]
mod terminal_select_tests {
    use super::*;
    fn apply(body: &str) -> String {
        let mut store = LineStore::new(format!(
            ".text\nf:\n.cfi_startproc\n# LCCC_RET_RDX 0\n{body}\n.cfi_endproc\n"
        ));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        run(&mut store, &mut infos);
        store.build_result(|i| infos[i].is_nop())
    }
    #[test]
    fn terminal_select_commutes_all_conditions_without_touching_test_flags() {
        for cc in [
            "e", "ne", "l", "ge", "le", "g", "b", "ae", "be", "a", "s", "ns", "o", "no", "p", "np",
        ] {
            for (w, s, d, a) in [("l", "%edx", "%ecx", "%eax"), ("q", "%r9", "%r8", "%rax")] {
                let out = apply(&format!(
                    "testl %edi, %edi\ncmov{cc}{w} {s}, {d}\nmov{w} {d}, {a}\nret"
                ));
                assert!(out.contains("testl %edi, %edi"), "{out}");
                assert!(out.contains(&format!("mov{w} {s}, {a}")), "{out}");
                assert!(
                    out.contains(&format!("cmov{}{w} {d}, {a}", invert_cc(cc))),
                    "{out}"
                );
            }
        }
    }
    #[test]
    fn terminal_select_keeps_memory_width_barriers_and_other_return_registers() {
        for body in [
            "cmovnel (%rdi), %ecx\nmovl %ecx, %eax\nret",
            "cmovneq %rdx, %rcx\nmovl %ecx, %eax\nret",
            "cmovnel %edx, %ecx\n.Lentry:\nmovl %ecx, %eax\nret",
            "cmovnel %esi, %edx\n# LCCC_RET_RDX 1\nmovl %edx, %eax\nret",
        ] {
            let out = apply(body);
            assert!(out.contains(body), "{out}");
        }
    }
    #[test]
    fn negated_arm_is_computed_directly_in_return_home() {
        for (w, s, t, a) in [("l", "%edi", "%r9d", "%eax"), ("q", "%rdi", "%rdx", "%rax")] {
            let out = apply(&format!(
                "mov{w} {s}, {t}\nneg{w} {t}\ntestl %esi, %esi\nmov{w} {t}, {a}\ncmove{w} {s}, {a}\nret"
            ));
            assert!(out.contains(&format!("mov{w} {s}, {a}")), "{out}");
            assert!(out.contains(&format!("neg{w} {a}")), "{out}");
            assert!(!out.contains(&format!("mov{w} {t}, {a}")), "{out}");
        }
    }
    #[test]
    fn negated_arm_cannot_change_an_early_result_read_or_shared_temporary() {
        for mid in [
            "testl %eax, %eax",
            "testl %r9d, %r9d",
            "addl $1, %esi",
            ".Lentry:",
            ".cfi_def_cfa_offset 16",
        ] {
            let body = format!("movl %edi, %r9d\nnegl %r9d\n{mid}\nmovl %r9d, %eax\nret");
            assert!(apply(&body).contains(&body), "{mid}");
        }
        let body =
            "movl %edi, %r9d\nnegl %r9d\ntestl %esi, %esi\nmovl %r9d, %eax\naddl %r9d, (%rdi)\nret";
        assert!(apply(body).contains(body));
    }
}
