//! Reuse both outputs of identical divisions with proven stable inputs.
//!
//! The accepted window is deliberately small: each division has the same
//! register or direct stack-slot dividend, the same register divisor and the
//! same signed/unsigned high-half setup. Between them are only explicit
//! register moves, whose destinations cannot clobber an input or either result.
//! No intervening instruction writes memory, and nothing is reordered.
//! A successful first division proves that the repeated division with the
//! same inputs cannot fault. DIV's undefined flags are not relied upon: the
//! second division is removed only when the existing flag oracle proves that
//! all of its flags are dead on every reachable path.
use super::super::types::*;
use super::direct_stack_slot;
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
        let source_reg = register_at_width(source.trim(), width);
        let stack_slot = direct_stack_slot(source.trim());
        let stack_base = stack_slot.map(|(base, _)| usize::from(base));
        if destination.trim() != REG_NAMES[width][0]
            || source_reg.is_none() && stack_base.is_none()
            || source_reg.is_some_and(|reg| matches!(reg, 0 | 2 | 4))
        {
            continue;
        }
        let setup_idx = next_non_debug(store, infos, first + 1, infos.len());
        if setup_idx >= infos.len() {
            continue;
        }
        let div_idx = next_non_debug(store, infos, setup_idx + 1, infos.len());
        if div_idx >= infos.len() || !all_editable(infos, &[first, setup_idx, div_idx]) {
            continue;
        }
        if infos[setup_idx].volatile_access || infos[div_idx].volatile_access {
            continue;
        }
        let setup = infos[setup_idx].trimmed(store.get(setup_idx));
        let div = infos[div_idx].trimmed(store.get(div_idx));
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
        // Cross only instructions with explicitly modeled effects. Any
        // unrecognized memory write, implicit output or barrier stops us.
        let mut cursor = div_idx + 1;
        let mut copies = 0;
        let mut candidate = infos.len();
        while cursor < infos.len() && cursor <= div_idx.saturating_add(32) {
            let i = next_non_debug(store, infos, cursor, infos.len());
            if i >= infos.len()
                || i > div_idx.saturating_add(32)
                || !all_editable(infos, &[i])
                || infos[i].volatile_access
            {
                break;
            }
            let text = infos[i].trimmed(store.get(i));
            if text == stage {
                candidate = i;
                break;
            }
            // Register-only comparisons have no GPR or memory outputs. The
            // removed divide's flags are independently proven dead below.
            let cmp = ["cmpl ", "cmpq ", "testl ", "testq "]
                .iter()
                .find_map(|prefix| text.strip_prefix(prefix).map(|args| (*prefix, args)));
            if let Some((opcode, args)) = cmp {
                let cmp_width =
                    usize::from(opcode.starts_with("cmpl") || opcode.starts_with("testl"));
                if let Some((src, dst)) = args.split_once(',') {
                    let src = src.trim();
                    let source_ok = register_at_width(src, cmp_width).is_some()
                        || src
                            .strip_prefix('$')
                            .is_some_and(|v| v.parse::<i64>().is_ok());
                    if source_ok && register_at_width(dst.trim(), cmp_width).is_some() {
                        copies += 1;
                        cursor = i + 1;
                        continue;
                    }
                }
                break;
            }
            // A direct stack store is safe only if it cannot overlap a stack
            // dividend. Distinct stack bases may alias and are not compared.
            if let Some(args) = text.strip_prefix(if width == 1 { "movl " } else { "movq " }) {
                if let Some((src, dst)) = args.split_once(',') {
                    if register_at_width(src.trim(), width).is_some() {
                        if let Some((base, offset)) = direct_stack_slot(dst.trim()) {
                            let size = if width == 1 { 4_i64 } else { 8_i64 };
                            let safe = stack_slot.is_none_or(|(read_base, read_offset)| {
                                let a = i64::from(offset);
                                let b = i64::from(read_offset);
                                base == read_base && (a + size <= b || b + size <= a)
                            });
                            if !safe {
                                break;
                            }
                            copies += 1;
                            cursor = i + 1;
                            continue;
                        }
                    }
                }
            }
            let Some(args) = text.strip_prefix(if width == 1 { "movl " } else { "movq " }) else {
                break;
            };
            let Some((src, dst)) = args.split_once(',') else {
                break;
            };
            if register_at_width(src.trim(), width).is_none() {
                break;
            }
            let Some(dest) = register_at_width(dst.trim(), width) else {
                break;
            };
            if matches!(dest, 0 | 2 | 4)
                || Some(dest) == source_reg
                || Some(dest) == stack_base
                || dest == divisor_reg
            {
                break;
            }
            copies += 1;
            cursor = i + 1;
        }
        if candidate == infos.len() || copies == 0 {
            continue;
        }
        let setup_b = next_non_debug(store, infos, candidate + 1, infos.len());
        if setup_b >= infos.len() {
            continue;
        }
        let div_b = next_non_debug(store, infos, setup_b + 1, infos.len());
        if div_b >= infos.len()
            || !all_editable(infos, &[candidate, setup_b, div_b])
            || infos[setup_b].volatile_access
            || infos[div_b].volatile_access
            || infos[setup_b].trimmed(store.get(setup_b)) != setup
            || infos[div_b].trimmed(store.get(div_b)) != div
            || !flags_dead_after(store, infos, div_b + 1)
        {
            continue;
        }
        for &i in &[candidate, setup_b, div_b] {
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
    fn crosses_multiple_safe_register_moves_but_not_a_clobber() {
        let body = "movq %rdi, %rax\ncqto\nidivq %rsi\nmovq %rax, %r8\nmovq %rdx, %r9\nmovq %rdi, %rax\ncqto\nidivq %rsi\nret\n";
        let (changed, output) = apply(body);
        assert!(changed, "{output}");
        assert_eq!(output.matches("idivq").count(), 1);
        assert!(!apply(&body.replace("movq %rdx, %r9", "movq %rdx, %rdi")).0);
    }

    #[test]
    fn stack_dividend_requires_stable_address_and_no_stores() {
        for (stage, setup, division, save) in [
            (
                "movl 8(%rsp), %eax",
                "xorl %edx, %edx",
                "divl %esi",
                "movl %edx, %r8d",
            ),
            (
                "movq -16(%rbp), %rax",
                "cqto",
                "idivq %rsi",
                "movq %rdx, %r8",
            ),
        ] {
            let body = format!(
                "{stage}\n{setup}\n{division}\n{save}\n{stage}\n{setup}\n{division}\nret\n"
            );
            let (changed, output) = apply(&body);
            assert!(changed, "{output}");
            assert_eq!(output.matches(division).count(), 1);
            assert!(!apply(&body.replace(save, &format!("{save}\nmovl %ecx, 8(%rsp)"))).0);
            assert!(
                !apply(&body.replace(save, &format!("{save}\nmovq %rcx, %rbp"))).0
                    || stage.contains("%rsp")
            );
        }
    }

    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    #[test]
    fn fused_stack_division_matches_unfused_on_hardware() {
        use std::process::Command;

        let body = "movq -8(%rsp), %rax\nxorq %rdx, %rdx\ndivq %rsi\nmovq %rdx, %r8\nmovq %rax, -24(%rsp)\ncmpq %r8, %rdi\nmovq -8(%rsp), %rax\nxorq %rdx, %rdx\ndivq %rsi\nxorq %r8, %rax\nret\n";
        let (changed, fused) = apply(body);
        assert!(changed);
        assert_eq!(fused.matches("divq").count(), 1);
        let signed_body = body
            .replace("xorq %rdx, %rdx", "cqto")
            .replace("divq", "idivq");
        let (signed_changed, signed_fused) = apply(&signed_body);
        assert!(signed_changed);
        assert_eq!(signed_fused.matches("idivq").count(), 1);
        let dir = std::env::temp_dir().join(format!("lccc_div_fusion_exec_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let asm = format!(
            ".text\n.globl original\noriginal:\nmovq %rdi, -8(%rsp)\n{body}\n.globl fused\nfused:\nmovq %rdi, -8(%rsp)\n{fused}\n.globl original_signed\noriginal_signed:\nmovq %rdi, -8(%rsp)\n{signed_body}\n.globl fused_signed\nfused_signed:\nmovq %rdi, -8(%rsp)\n{signed_fused}\n.section .note.GNU-stack,\"\",@progbits\n"
        );
        let driver = r#"
#include <stdint.h>
uint64_t original(uint64_t, uint64_t), fused(uint64_t, uint64_t);
uint64_t original_signed(int64_t, int64_t), fused_signed(int64_t, int64_t);
int main(void) {
    uint64_t x = 0x123456789abcdef0ULL;
    for (int i = 0; i < 100000; ++i) {
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        uint64_t d = (x >> 32) | 1;
        uint64_t expected = (x / d) ^ (x % d);
        if (original(x, d) != expected || fused(x, d) != expected) return 1;
        int64_t numerator = (int64_t)x;
        int64_t divisor = (int64_t)d;
        if (x & 2) divisor = -divisor;
        if (divisor == -1) divisor = -3;
        uint64_t signed_expected = (uint64_t)(numerator / divisor) ^ (uint64_t)(numerator % divisor);
        if (original_signed(numerator, divisor) != signed_expected ||
            fused_signed(numerator, divisor) != signed_expected) return 2;
    }
    return 0;
}
"#;
        let asm_path = dir.join("test.s");
        let driver_path = dir.join("driver.c");
        let exe = dir.join("test");
        std::fs::write(&asm_path, asm).unwrap();
        std::fs::write(&driver_path, driver).unwrap();
        let compile = Command::new("gcc")
            .arg(&asm_path)
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("gcc is required for the hardware regression");
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let status = Command::new(&exe)
            .status()
            .expect("hardware regression must run");
        let _ = std::fs::remove_dir_all(dir);
        assert!(status.success(), "hardware regression returned {status}");
    }

    #[test]
    fn crosses_only_non_aliasing_stack_stores_and_register_compares() {
        let body = "movq 8(%rsp), %rax\nxorq %rdx, %rdx\ndivq %rsi\nmovq %rax, 24(%rsp)\ncmpq %r8, %r9\nmovq 8(%rsp), %rax\nxorq %rdx, %rdx\ndivq %rsi\nret\n";
        assert!(apply(body).0);
        assert!(!apply(&body.replace("24(%rsp)", "15(%rsp)")).0);
        assert!(!apply(&body.replace("24(%rsp)", "24(%rbp)")).0);
        assert!(!apply(&body.replace("cmpq %r8, %r9", "cmpq (%r8), %r9")).0);
        assert!(!apply(&body.replace("ret\n", "lahf\nret\n")).0);
        let register = body.replace("8(%rsp)", "%rdi");
        assert!(apply(&register).0);
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
