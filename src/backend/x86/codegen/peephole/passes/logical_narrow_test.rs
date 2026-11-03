//! Delete an adjacent same-width byte/word TEST after AND, OR or XOR.
//! Both instructions set SF/ZF/PF identically, clear CF/OF and leave AF
//! undefined. Exact register spelling matters: AH and AL are not aliases.
use super::super::types::*;

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for i in 0..infos.len() {
        if infos[i].is_nop() || infos[i].pinned || infos[i].volatile_access {
            continue;
        }
        let text = infos[i].trimmed(store.get(i));
        let Some((opcode, operands)) = text.split_once(' ') else {
            continue;
        };
        let test = match opcode {
            "andb" | "orb" | "xorb" => "testb",
            "andw" | "orw" | "xorw" => "testw",
            _ => continue,
        };
        if operands.contains(['#', ';', '\n']) {
            continue;
        }
        let Some((_, destination)) = operands.rsplit_once(',') else {
            continue;
        };
        let destination = destination.trim();
        let valid = if test == "testb" {
            REG_NAMES[3].contains(&destination)
                || ["%ah", "%bh", "%ch", "%dh"].contains(&destination)
        } else {
            REG_NAMES[2].contains(&destination)
        };
        if !valid {
            continue;
        }
        let j = next_non_debug(store, infos, i + 1, infos.len());
        if j == infos.len() || infos[j].volatile_access || !all_editable(infos, &[i, j]) {
            continue;
        }
        if infos[j].trimmed(store.get(j)) != format!("{test} {destination}, {destination}") {
            continue;
        }
        mark_nop(&mut infos[j]);
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
    fn same_width_and_exact_byte_alias_only() {
        for body in [
            "andb $1, %al\ntestb %al, %al\njs .L1\n.L1:\nret\n",
            "orb %bh, %ah\ntestb %ah, %ah\nret\n",
            "xorw %r9w, %r8w\ntestw %r8w, %r8w\nret\n",
        ] {
            let (changed, output) = apply(body);
            assert!(changed && !output.contains("test"), "{output}");
        }
        for body in [
            "andb $1, %ah\ntestb %al, %al\nret\n",
            "andw $1, %ax\ntestl %eax, %eax\nret\n",
            "andb $1, %al\n.byte 0x9e\ntestb %al, %al\nret\n",
            "#APP\nandb $1, %al\ntestb %al, %al\n#NO_APP\nret\n",
        ] {
            let (changed, output) = apply(body);
            assert!(!changed, "{output}");
        }
    }
}
