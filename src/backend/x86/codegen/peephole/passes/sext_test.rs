//! Fuse a dead same-register sign extension and its self-test.
//!
//! Sign extension preserves zero, sign and low-byte parity. TEST clears CF
//! and OF and leaves AF undefined at either width. The extended register must
//! be dead on every successor: the narrow TEST does not write it.
use super::super::types::*;
use super::liveness::FileLiveness;

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut liveness = None;
    let mut changed = false;
    for i in 0..infos.len() {
        if infos[i].is_nop() || infos[i].volatile_access || !all_editable(infos, &[i]) {
            continue;
        }
        let text = infos[i].trimmed(store.get(i));
        let Some((op, args)) = text.split_once(' ') else {
            continue;
        };
        let (source_width, dest_width, test) = match op {
            "movsbl" => (3, 1, "testl"),
            "movswl" => (2, 1, "testl"),
            "movsbq" => (3, 0, "testq"),
            "movswq" => (2, 0, "testq"),
            _ => continue,
        };
        let Some((src, dst)) = args.split_once(',') else {
            continue;
        };
        let (src, dst) = (src.trim(), dst.trim());
        let Some(fam) = REG_NAMES[dest_width].iter().position(|&name| name == dst) else {
            continue;
        };
        if src != REG_NAMES[source_width][fam] || fam == REG_RSP as usize {
            continue;
        }
        let j = next_non_debug(store, infos, i + 1, infos.len());
        if j >= infos.len() || infos[j].volatile_access || !all_editable(infos, &[i, j]) {
            continue;
        }
        if infos[j].trimmed(store.get(j)) != format!("{test} {dst}, {dst}") {
            continue;
        }
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        if lv.live_after(j, fam as RegId) != Some(false) {
            continue;
        }
        let narrow_test = if source_width == 3 { "testb" } else { "testw" };
        let replacement = format!("    {narrow_test} {src}, {src}");
        replace_line(store, &mut infos[i], i, replacement);
        mark_nop(&mut infos[j]);
        lv.refresh_span(store, infos, i, j);
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
    fn narrow_test_requires_a_dead_extended_result() {
        let body = ".cfi_startproc\nmovsbl %cl, %ecx\ntestl %ecx, %ecx\nje .Lout\n.Lout:\nret\n.cfi_endproc\n";
        let (changed, result) = apply(body);
        assert!(changed, "{result}");
        assert!(result.contains("testb %cl, %cl"), "{result}");
        assert!(!result.contains("movsbl"), "{result}");

        for (extension, test, narrow) in [
            ("movswl %cx, %ecx", "testl %ecx, %ecx", "testw %cx, %cx"),
            ("movsbq %r8b, %r8", "testq %r8, %r8", "testb %r8b, %r8b"),
            ("movswq %r9w, %r9", "testq %r9, %r9", "testw %r9w, %r9w"),
        ] {
            let body = format!(".cfi_startproc\n{extension}\n{test}\nret\n.cfi_endproc\n");
            let (changed, result) = apply(&body);
            assert!(changed && result.contains(narrow), "{result}");
        }

        for body in [
            ".cfi_startproc\nmovsbl %cl, %ecx\ntestl %ecx, %ecx\nmovl %ecx, %eax\nret\n.cfi_endproc\n",
            ".cfi_startproc\nmovsbl %cl, %ecx\ntestq %rcx, %rcx\nret\n.cfi_endproc\n",
            ".cfi_startproc\nmovsbl %ah, %eax\ntestl %eax, %eax\nret\n.cfi_endproc\n",
            ".cfi_startproc\n#APP\nmovsbl %cl, %ecx\ntestl %ecx, %ecx\n#NO_APP\nret\n.cfi_endproc\n",
        ] {
            let (changed, result) = apply(body);
            assert!(!changed, "{result}");
        }
    }
}
