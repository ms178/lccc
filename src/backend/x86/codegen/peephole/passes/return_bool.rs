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
