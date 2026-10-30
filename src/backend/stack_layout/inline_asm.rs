//! Inline asm callee-saved register scanning.
//!
//! Scan functions for inline asm instructions and identify which callee-saved
//! registers are clobbered. Three variants handle different architectures:
//! - basic: x86 and RISC-V (explicit register constraints)
//! - with_overflow: x86 (overflow into callee-saved scratch pool)
//! - with_generic: i686 (conservative marking for generic constraints)

use super::super::regalloc::PhysReg;
use crate::common::fx_hash::FxHashSet;
use crate::ir::reexports::{Instruction, IrFunction};

// ── Inline asm callee-saved scanning ──────────────────────────────────────

/// Scan inline-asm instructions for callee-saved register usage.
///
/// Iterates over all inline-asm instructions in `func`, checking output/input
/// constraints and clobber lists for callee-saved registers.  Uses two callbacks:
/// - `constraint_to_phys`: maps an output/input constraint string to a PhysReg
/// - `clobber_to_phys`: maps a clobber register name to a PhysReg
///
/// Any discovered callee-saved PhysRegs are appended to `used` (deduplicated).
/// This shared helper eliminates duplicated scan loops in x86 and RISC-V.
///
/// When a generic register class constraint (e.g. "r", "q", "g") is found that
/// doesn't map to a specific register, ALL callee-saved registers from
/// `all_callee_saved` are conservatively marked as clobbered. This prevents the
/// register allocator from assigning callee-saved registers to values whose live
/// ranges span the inline asm block, since the scratch register allocator may
/// pick any callee-saved register at codegen time. This is especially important
/// on i686 where there are only 3 callee-saved GP registers (ebx, esi, edi) and
/// the scratch pool includes all of them.
pub fn collect_inline_asm_callee_saved(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
) {
    collect_inline_asm_callee_saved_inner(func, used, constraint_to_phys, clobber_to_phys, &[])
}

/// Like `collect_inline_asm_callee_saved`, but triggers conservative marking
/// of all callee-saved registers whenever an inline asm's worst-case GP scratch
/// demand (including memory-operand address staging and register pairs) exceeds
/// the remaining un-clobbered caller-saved GP scratch pool.
#[allow(clippy::too_many_arguments)]
pub fn collect_inline_asm_callee_saved_with_overflow(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    constraint_to_caller_scratch: impl Fn(&str) -> Option<usize>,
    clobber_to_caller_scratch: impl Fn(&str) -> Option<usize>,
    is_specific_non_scratch_constraint: impl Fn(&str) -> bool,
    all_callee_saved: &[PhysReg],
    caller_saved_scratch_count: usize,
) {
    use crate::common::types::IrType;

    fn is_non_gp_scratch_constraint(c: &str) -> bool {
        !c.is_empty()
            && c.chars().all(|ch| {
                matches!(
                    ch,
                    'i' | 'n'
                        | 'I'
                        | 'J'
                        | 'K'
                        | 'L'
                        | 'M'
                        | 'N'
                        | 'O'
                        | 'e'
                        | 'Z'
                        | 's'
                        | 'x'
                        | 'v'
                        | 'Y'
                        | 'w'
                        | 'f'
                        | 't'
                        | 'u'
                )
            })
    }

    let mut already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm {
                outputs,
                inputs,
                clobbers,
                operand_types,
                ..
            } = inst
            {
                let mut generic_gp_demand = 0usize;
                let mut claimed_caller_scratch: u64 = 0;
                let mut force_callee_saved = false;
                let num_plus = outputs.iter().filter(|(c, _, _)| c.contains('+')).count();

                for (op_idx, (constraint, _, _)) in outputs.iter().enumerate() {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if c.starts_with("@cc")
                        || (!c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()))
                        || is_non_gp_scratch_constraint(c)
                    {
                        continue;
                    }
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if let Some(slot) = constraint_to_caller_scratch(c) {
                        if slot < 64 {
                            claimed_caller_scratch |= 1u64 << slot;
                        }
                    } else if is_specific_non_scratch_constraint(c) || c.starts_with('{') {
                        // Explicit non-scratch register (e.g. "a" / "{rax}")
                    } else {
                        if c.contains('Q') {
                            // x86 "Q" draws from [rax, rbx, rcx, rdx], reaching callee-saved rbx directly.
                            force_callee_saved = true;
                        }
                        let is_pair = matches!(
                            operand_types.get(op_idx),
                            Some(IrType::I128) | Some(IrType::U128)
                        );
                        generic_gp_demand += if is_pair { 2 } else { 1 };
                    }
                }
                for (idx, (constraint, _, _)) in inputs.iter().enumerate() {
                    let op_idx = outputs.len() + idx;
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if c.starts_with("@cc")
                        || (!c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()))
                        || is_non_gp_scratch_constraint(c)
                    {
                        continue;
                    }
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if let Some(slot) = constraint_to_caller_scratch(c) {
                        if slot < 64 {
                            claimed_caller_scratch |= 1u64 << slot;
                        }
                    } else if is_specific_non_scratch_constraint(c) || c.starts_with('{') {
                        // Explicit non-scratch register
                    } else if idx >= num_plus {
                        if c.contains('Q') {
                            force_callee_saved = true;
                        }
                        let is_pair = matches!(
                            operand_types.get(op_idx),
                            Some(IrType::I128) | Some(IrType::U128)
                        );
                        generic_gp_demand += if is_pair { 2 } else { 1 };
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    }
                    if let Some(slot) = clobber_to_caller_scratch(clobber.as_str()) {
                        if slot < 64 {
                            claimed_caller_scratch |= 1u64 << slot;
                        }
                    }
                }
                let available_scratch = caller_saved_scratch_count
                    .saturating_sub(claimed_caller_scratch.count_ones() as usize);
                if force_callee_saved || generic_gp_demand > available_scratch {
                    for &phys in all_callee_saved {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    }
                }
            }
        }
    }
}

/// Like `collect_inline_asm_callee_saved`, but with an additional
/// `all_callee_saved` list. When a constraint is a generic GP register class
/// (like "r", "q", "g") that doesn't map to a specific callee-saved register,
/// all registers in `all_callee_saved` are conservatively marked as clobbered.
pub fn collect_inline_asm_callee_saved_with_generic(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
) {
    collect_inline_asm_callee_saved_inner(
        func,
        used,
        constraint_to_phys,
        clobber_to_phys,
        all_callee_saved,
    )
}

/// Returns true if the constraint string (after stripping `=`, `+`, `&`)
/// contains a generic GP register class character that could cause the scratch
/// allocator to pick any GP register, including callee-saved ones.
fn is_generic_gp_constraint(constraint: &str) -> bool {
    // Skip explicit register constraints like {eax}
    if constraint.starts_with('{') {
        return false;
    }
    // Skip tied operands (all digits)
    if !constraint.is_empty() && constraint.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    // Skip condition code constraints (@cc...)
    if constraint.starts_with("@cc") {
        return false;
    }
    // Check for generic GP register class characters
    for ch in constraint.chars() {
        match ch {
            // 'r', 'q', 'R', 'Q', 'l' = any GP register
            // 'g' = general operand (GP reg, memory, or immediate)
            'r' | 'q' | 'R' | 'Q' | 'l' | 'g' => return true,
            // Specific register letters are not generic
            'a' | 'b' | 'c' | 'd' | 'S' | 'D' => return false,
            _ => {}
        }
    }
    false
}

/// i686-precision variant: skip the blanket callee-saved marking when every
/// inline-asm block's worst-case scratch demand fits in the caller-saved
/// prefix of the i686 scratch pool.
///
/// The i686 scratch allocator (asm_emitter.rs) hands out registers in the
/// fixed order [ecx, edx, esi, edi, eax, ebx], skipping excluded entries.
/// Therefore, if an asm block can consume at most D scratch registers and
/// the first two pool entries (ecx, edx) are not excluded by a specific
/// constraint or clobber, then D <= avail({ecx,edx}) proves no callee-saved
/// register can ever be handed out for that block — the blanket marking
/// (which costs push/pop pairs AND starves the register allocator in every
/// function with a segment-register read, i.e. all of arch/x86/boot) is
/// dropped.
///
/// Demand counting is deliberately over-approximate: every operand that is
/// not a tied digit, an immediate-only class, a directly stored byte @cc
/// output, or an explicit single-register letter counts as one scratch
/// register (two if its operand type is 64-bit, since i686 uses a register
/// pair). Memory operands count too (address staging draws from the same pool).
pub fn collect_inline_asm_callee_saved_i686(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
) {
    use crate::common::types::IrType;

    // Specific single-register constraint letters on i686.
    fn specific_reg_letter(c: &str) -> Option<&'static str> {
        match c {
            "a" => Some("eax"),
            "b" => Some("ebx"),
            "c" => Some("ecx"),
            "d" => Some("edx"),
            "S" => Some("esi"),
            "D" => Some("edi"),
            _ => None,
        }
    }
    // Immediate-only constraint (no register alternative)?
    fn imm_only(c: &str) -> bool {
        !c.is_empty()
            && c.chars().all(|ch| {
                matches!(
                    ch,
                    'i' | 'n' | 'I' | 'J' | 'K' | 'L' | 'M' | 'N' | 'O' | 'e' | 'Z' | 's'
                )
            })
    }

    let alloca_ids: FxHashSet<u32> = func
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .filter_map(|inst| match inst {
            Instruction::Alloca { dest, .. } => Some(dest.0),
            _ => None,
        })
        .collect();
    let mut all_blocks_fit = true;
    'scan: for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::InlineAsm {
                outputs,
                inputs,
                clobbers,
                operand_types,
                ..
            } = inst
            else {
                continue;
            };

            let mut avail: i32 = 2; // ecx, edx
            let mut ecx_gone = false;
            let mut edx_gone = false;
            let mut demand: i32 = 0;
            let mut op_idx = 0usize;

            let account = |c: &str,
                           op_idx: usize,
                           ecx_gone: &mut bool,
                           edx_gone: &mut bool,
                           avail: &mut i32,
                           demand: &mut i32| {
                let c = c.trim_start_matches(['=', '+', '&', '%']);
                if c.starts_with("@cc") {
                    let direct_byte_store =
                        matches!(operand_types.get(op_idx), Some(IrType::I8 | IrType::U8))
                            && outputs
                                .get(op_idx)
                                .is_some_and(|(_, ptr, _)| alloca_ids.contains(&ptr.0));
                    if !direct_byte_store {
                        *demand += 1;
                    }
                    return;
                }
                if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) {
                    return;
                } // tied
                if imm_only(c) {
                    return;
                }
                if let Some(reg) = specific_reg_letter(c) {
                    if reg == "ecx" && !*ecx_gone {
                        *ecx_gone = true;
                        *avail -= 1;
                    }
                    if reg == "edx" && !*edx_gone {
                        *edx_gone = true;
                        *avail -= 1;
                    }
                    return; // no scratch draw
                }
                let wide = matches!(
                    operand_types.get(op_idx),
                    Some(IrType::I64) | Some(IrType::U64) | Some(IrType::F64)
                );
                *demand += if wide { 2 } else { 1 };
            };

            for (c, _, _) in outputs {
                account(
                    c,
                    op_idx,
                    &mut ecx_gone,
                    &mut edx_gone,
                    &mut avail,
                    &mut demand,
                );
                op_idx += 1;
            }
            for (c, _, _) in inputs {
                account(
                    c,
                    op_idx,
                    &mut ecx_gone,
                    &mut edx_gone,
                    &mut avail,
                    &mut demand,
                );
                op_idx += 1;
            }
            for cl in clobbers {
                let name = crate::backend::inline_asm::normalize_inline_asm_reg_name(cl);
                if matches!(name, "ecx" | "cx" | "cl" | "ch") && !ecx_gone {
                    ecx_gone = true;
                    avail -= 1;
                }
                if matches!(name, "edx" | "dx" | "dl" | "dh") && !edx_gone {
                    edx_gone = true;
                    avail -= 1;
                }
            }

            let has_cc_output = outputs.iter().any(|(constraint, _, _)| {
                constraint
                    .trim_start_matches(['=', '+', '&', '%'])
                    .starts_with("@cc")
            });
            let has_register_output = outputs.iter().any(|(constraint, _, _)| {
                let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                c.contains('r') || c.contains('q') || c.contains('g')
            });
            let has_register_input = inputs.iter().any(|(constraint, _, _)| {
                let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                c.contains('r') || c.contains('q') || c.contains('g')
            });
            let caller_scratch_saturated = demand > 0 && demand >= avail.max(0);
            if demand > avail.max(0)
                || caller_scratch_saturated
                || (has_cc_output && has_register_output && has_register_input)
            {
                // Mixed flag and register outputs make the generic scratch
                // allocator's available caller-saved prefix depend on live RA
                // homes, which the early demand scan cannot see. Reserve the
                // callee-saved pool rather than risk an unsaved asm scratch
                // clobber on this uncommon shape.
                all_blocks_fit = false;
                break 'scan;
            }
        }
    }

    if all_blocks_fit {
        // Precise mode: only explicitly named callee-saved registers
        // (constraints/clobbers) are marked; generic classes draw from
        // the proven-sufficient ecx/edx prefix.
        collect_inline_asm_callee_saved_inner(func, used, constraint_to_phys, clobber_to_phys, &[]);
    } else {
        // Scratch demand (including memory-operand address staging) can overflow
        // the ecx/edx caller-saved prefix into esi/edi/ebx, so mark all callee-saved
        // registers even when no operand used a generic "r" letter (e.g. 3 "m" operands).
        collect_inline_asm_callee_saved_inner(
            func,
            used,
            constraint_to_phys,
            clobber_to_phys,
            all_callee_saved,
        );
        let already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
        for &phys in all_callee_saved {
            if !already.contains(&phys.0) {
                used.push(phys);
            }
        }
    }
}

fn collect_inline_asm_callee_saved_inner(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
) {
    let mut already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm {
                outputs,
                inputs,
                clobbers,
                ..
            } = inst
            {
                for (constraint, _, _) in outputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        // Generic register class: conservatively mark all
                        // callee-saved registers as clobbered since the
                        // scratch allocator may pick any of them.
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) {
                                used.push(phys);
                            }
                        }
                    }
                }
                for (constraint, _, _) in inputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) {
                                used.push(phys);
                            }
                        }
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, Operand, Terminator, Value};

    fn make_func_with_asm(inst: Instruction) -> IrFunction {
        let mut func = IrFunction::new("test_asm".to_string(), IrType::I64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![inst],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        func
    }

    #[test]
    fn overflow_tracks_caller_saved_clobbers_and_mem_operands() {
        let all_cs = [PhysReg(1), PhysReg(2), PhysReg(3), PhysReg(4), PhysReg(5)];
        let caller_slot = |name: &str| match name {
            "rcx" | "c" => Some(0),
            "rdx" | "d" => Some(1),
            "rsi" | "S" => Some(2),
            "rdi" | "D" => Some(3),
            "r8" => Some(4),
            "r9" => Some(5),
            "r10" => Some(6),
            "r11" => Some(7),
            _ => None,
        };
        // Common 2-operand asm with no caller-saved clobbers fits in 8 caller-saved regs.
        let simple = make_func_with_asm(Instruction::InlineAsm {
            template: "movq %1, %0".to_string(),
            outputs: vec![("=r".to_string(), Value(0), None)],
            inputs: vec![("r".to_string(), Operand::Value(Value(1)), None)],
            clobbers: vec!["memory".to_string()],
            operand_types: vec![IrType::I64, IrType::I64],
            goto_labels: vec![],
            input_symbols: vec![None],
            seg_overrides: vec![AddressSpace::Default, AddressSpace::Default],
        });
        let mut used = Vec::new();
        collect_inline_asm_callee_saved_with_overflow(
            &simple,
            &mut used,
            |_| None,
            |_| None,
            caller_slot,
            caller_slot,
            |c| c == "a" || c == "A",
            &all_cs,
            8,
        );
        assert!(
            used.is_empty(),
            "simple 2-operand asm must not reserve callee-saved regs"
        );

        // Clobbering all 8 caller-saved scratch regs with a "+m" operand forces overflow.
        let overflow = make_func_with_asm(Instruction::InlineAsm {
            template: "incq %0".to_string(),
            outputs: vec![("+m".to_string(), Value(0), None)],
            inputs: vec![("m".to_string(), Operand::Value(Value(0)), None)],
            clobbers: vec!["rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"]
                .into_iter()
                .map(String::from)
                .collect(),
            operand_types: vec![IrType::I64, IrType::I64],
            goto_labels: vec![],
            input_symbols: vec![None],
            seg_overrides: vec![AddressSpace::Default, AddressSpace::Default],
        });
        used.clear();
        collect_inline_asm_callee_saved_with_overflow(
            &overflow,
            &mut used,
            |_| None,
            |_| None,
            caller_slot,
            caller_slot,
            |c| c == "a" || c == "A",
            &all_cs,
            8,
        );
        assert_eq!(used, all_cs);
    }

    #[test]
    fn i686_subreg_clobbers_and_mem_only_overflow_mark_callee_saved() {
        let all_cs = [PhysReg(1), PhysReg(2), PhysReg(3)];
        let mem_overflow = make_func_with_asm(Instruction::InlineAsm {
            template: "movl $1, %0; movl $2, %1".to_string(),
            outputs: vec![
                ("=m".to_string(), Value(0), None),
                ("=m".to_string(), Value(1), None),
            ],
            inputs: vec![],
            clobbers: vec!["cl".to_string(), "dl".to_string()],
            operand_types: vec![IrType::I32, IrType::I32],
            goto_labels: vec![],
            input_symbols: vec![],
            seg_overrides: vec![AddressSpace::Default, AddressSpace::Default],
        });
        let mut used = Vec::new();
        collect_inline_asm_callee_saved_i686(&mem_overflow, &mut used, |_| None, |_| None, &all_cs);
        assert_eq!(used, all_cs);
    }
}
