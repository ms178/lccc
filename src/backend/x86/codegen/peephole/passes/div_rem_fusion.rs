//! Reuse both outputs of identical, adjacent register-only divisions.
//!
//! The accepted window is deliberately small: each division has the same
//! register dividend, the same register divisor and the same signed/unsigned
//! high-half setup. Between them is exactly one register copy of a result,
//! whose destination cannot clobber an input or either result. There are no
//! memory loads or stores to alias and no instructions are reordered.
//! A successful first division proves that the repeated division with the
//! same inputs cannot fault. DIV's undefined flags are not relied upon: the
//! second division is removed only when the existing flag oracle proves that
//! all of its flags are dead on every reachable path.
use super::super::types::*;
use super::flag_peepholes::flags_dead_after;

fn register_at_width(text: &str, width: usize) -> Option<usize> {
    REG_NAMES[width].iter().position(|&name| name == text)
}

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for first in 0..infos.len() {
        if infos[first].is_nop() || infos[first].pinned || infos[first].volatile_access {
            continue;
        }
        let stage = infos[first].trimmed(store.get(first));
        let Some((opcode, operands)) = stage.split_once(' ') else {
            continue;
        };
        let width = match opcode {
            "movl" => 1,
            "movq" => 0,
            _ => continue,
        };
        let Some((source, destination)) = operands.split_once(',') else {
            continue;
        };
        let Some(source_reg) = register_at_width(source.trim(), width) else {
            continue;
        };
        if destination.trim() != REG_NAMES[width][0] || matches!(source_reg, 0 | 2 | 4) {
            continue;
        }
        let mut lines = [infos.len(); 7];
        lines[0] = first;
        for k in 1..lines.len() {
            lines[k] = next_non_debug(store, infos, lines[k - 1] + 1, infos.len());
            if lines[k] == infos.len() {
                break;
            }
        }
        if lines[6] >= infos.len() || !all_editable(infos, &lines) {
            continue;
        }
        if lines
            .iter()
            .any(|&i| infos[i].volatile_access || infos[i].is_nop())
        {
            continue;
        }
        let setup = infos[lines[1]].trimmed(store.get(lines[1]));
        let div = infos[lines[2]].trimmed(store.get(lines[2]));
        let signed = match (width, setup) {
            (1, "xorl %edx, %edx") | (0, "xorq %rdx, %rdx") => false,
            (1, "cltd" | "cdq") | (0, "cqto" | "cqo") => true,
            _ => continue,
        };
        let div_opcode = match (width, signed) {
            (1, false) => "divl ",
            (1, true) => "idivl ",
            (0, false) => "divq ",
            _ => "idivq ",
        };
        let Some(divisor) = div.strip_prefix(div_opcode) else {
            continue;
        };
        let Some(divisor_reg) = register_at_width(divisor.trim(), width) else {
            continue;
        };
        if matches!(divisor_reg, 0 | 2 | 4) {
            continue;
        }
        let result_copy = infos[lines[3]].trimmed(store.get(lines[3]));
        let Some(copy_args) = result_copy.strip_prefix(if width == 1 { "movl " } else { "movq " })
        else {
            continue;
        };
        let Some((copy_source, copy_dest)) = copy_args.split_once(',') else {
            continue;
        };
        if !matches!(copy_source.trim(), "%eax" | "%edx" | "%rax" | "%rdx")
            || ![REG_NAMES[width][0], REG_NAMES[width][2]].contains(&copy_source.trim())
        {
            continue;
        }
        let Some(copy_reg) = register_at_width(copy_dest.trim(), width) else {
            continue;
        };
        if matches!(copy_reg, 0 | 2 | 4) || copy_reg == source_reg || copy_reg == divisor_reg {
            continue;
        }
        if infos[lines[4]].trimmed(store.get(lines[4])) != stage
            || infos[lines[5]].trimmed(store.get(lines[5])) != setup
            || infos[lines[6]].trimmed(store.get(lines[6])) != div
            || !flags_dead_after(store, infos, lines[6] + 1)
        {
            continue;
        }
        for &i in &lines[4..] {
            mark_nop(&mut infos[i]);
        }
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
    fn fuses_identical_signed_and_unsigned_divisions() {
        for (stage, setup, div, copy) in [
            (
                "movl %edi, %eax",
                "xorl %edx, %edx",
                "divl %esi",
                "movl %edx, %r8d",
            ),
            ("movl %edi, %eax", "cltd", "idivl %esi", "movl %eax, %r8d"),
            (
                "movq %rdi, %rax",
                "xorq %rdx, %rdx",
                "divq %rsi",
                "movq %rdx, %r8",
            ),
            ("movq %rdi, %rax", "cqto", "idivq %rsi", "movq %rax, %r8"),
        ] {
            let body = format!("{stage}\n{setup}\n{div}\n{copy}\n{stage}\n{setup}\n{div}\nret\n");
            let (changed, output) = apply(&body);
            assert!(changed, "{output}");
            assert_eq!(output.matches(div).count(), 1, "{output}");
            assert!(output.contains(copy), "{output}");
        }
    }

    #[test]
    fn every_register_alias_is_checked() {
        for width in 0..=1 {
            let names = &REG_NAMES[width];
            let (mov, setup, div) = if width == 0 {
                ("movq", "xorq %rdx, %rdx", "divq")
            } else {
                ("movl", "xorl %edx, %edx", "divl")
            };
            for source in 0..16 {
                for divisor in 0..16 {
                    for copy in 0..16 {
                        let stage = format!("{mov} {}, {}", names[source], names[0]);
                        let division = format!("{div} {}", names[divisor]);
                        let save = format!("{mov} {}, {}", names[2], names[copy]);
                        let body = format!(
                            "{stage}\n{setup}\n{division}\n{save}\n{stage}\n{setup}\n{division}\nret\n"
                        );
                        let expected = !matches!(source, 0 | 2 | 4)
                            && !matches!(divisor, 0 | 2 | 4)
                            && !matches!(copy, 0 | 2 | 4)
                            && copy != source
                            && copy != divisor;
                        let (changed, output) = apply(&body);
                        assert_eq!(changed, expected, "{body} => {output}");
                    }
                }
            }
        }
    }

    #[test]
    fn refuses_changed_inputs_flags_and_opaque_code() {
        for body in [
            "movl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %edi\nmovl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nret\n",
            "movl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %esi\nmovl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nret\n",
            "movl %eax, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %r8d\nmovl %eax, %eax\nxorl %edx, %edx\ndivl %esi\nret\n",
            "movl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %r8d\nmovl %edi, %eax\nxorl %edx, %edx\nidivl %esi\nret\n",
            "movl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %r8d\nmovl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nlahf\nret\n",
            "#APP\nmovl %edi, %eax\nxorl %edx, %edx\ndivl %esi\nmovl %edx, %r8d\nmovl %edi, %eax\nxorl %edx, %edx\ndivl %esi\n#NO_APP\nret\n",
            "movl %edi, %eax\nxorl %edx, %edx\ndivl %esi\n",
        ] {
            let (changed, output) = apply(body);
            assert!(!changed, "{output}");
        }
    }
}
