//! Inline asm callee-saved register scanning.
//!
//! Scan functions for inline asm instructions and identify which callee-saved
//! registers are clobbered. Three variants handle different architectures:
//! - basic: x86 and RISC-V (explicit register constraints)
//! - with_overflow: x86 (overflow into callee-saved scratch pool)
//! - with_generic: i686 (conservative marking for generic constraints)

use crate::ir::reexports::{Instruction, IrFunction};
use crate::common::fx_hash::FxHashSet;
use super::super::regalloc::PhysReg;

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
/// of all callee-saved registers when any single inline asm has more generic GP
/// register operands than the caller-saved scratch pool can hold. This avoids
/// the overly conservative behavior of `_with_generic` (which marks all callee-saved
/// for ANY generic "r" constraint, even if only 1 register is needed).
pub fn collect_inline_asm_callee_saved_with_overflow(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
    caller_saved_scratch_count: usize,
) {
    let mut already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm { outputs, inputs, clobbers, .. } = inst {
                // Count how many generic GP register operands this inline asm has.
                // This approximates how many scratch registers will be needed.
                // Note: "+" outputs create synthetic inputs too, but those are handled
                // by finalize (not scratch alloc), so we only count non-plus outputs
                // and non-synthetic inputs for the overflow check.
                let mut generic_gp_count = 0usize;
                let mut specific_claimed = 0usize;
                let mut clobber_claimed = 0usize;
                let num_plus = outputs.iter().filter(|(c, _, _)| c.contains('+')).count();

                for (constraint, _, _) in outputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) { used.push(phys); }
                        specific_claimed += 1;
                    } else if is_generic_gp_constraint(c) {
                        generic_gp_count += 1;
                    }
                }
                for (idx, (constraint, _, _)) in inputs.iter().enumerate() {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) { used.push(phys); }
                        specific_claimed += 1;
                    } else if idx >= num_plus && is_generic_gp_constraint(c) {
                        // Only count non-synthetic inputs (synthetic "+" inputs
                        // don't consume scratch registers)
                        generic_gp_count += 1;
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) { used.push(phys); }
                        clobber_claimed += 1;
                    }
                }
                // If generic GP operands exceed the available caller-saved scratch
                // pool (after accounting for specific/clobber claims), the scratch
                // allocator will overflow into callee-saved registers.
                let available_scratch = caller_saved_scratch_count.saturating_sub(specific_claimed + clobber_claimed);
                if generic_gp_count > available_scratch {
                    for &phys in all_callee_saved {
                        if already.insert(phys.0) { used.push(phys); }
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
    collect_inline_asm_callee_saved_inner(func, used, constraint_to_phys, clobber_to_phys, all_callee_saved)
}

/// Returns true if the constraint string (after stripping `=`, `+`, `&`)
/// contains a generic GP register class character that could cause the scratch
/// allocator to pick any GP register, including callee-saved ones.
fn is_generic_gp_constraint(constraint: &str) -> bool {
    // Skip explicit register constraints like {eax}
    if constraint.starts_with('{') { return false; }
    // Skip tied operands (all digits)
    if !constraint.is_empty() && constraint.chars().all(|ch| ch.is_ascii_digit()) { return false; }
    // Skip condition code constraints (@cc...)
    if constraint.starts_with("@cc") { return false; }
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
/// not a tied digit, an immediate-only class, an @cc flag output, or an
/// explicit single-register letter counts as one scratch register (two if
/// its operand type is 64-bit, since i686 uses a register pair). Memory
/// operands count too (address staging draws from the same pool).
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
            "a" => Some("eax"), "b" => Some("ebx"), "c" => Some("ecx"),
            "d" => Some("edx"), "S" => Some("esi"), "D" => Some("edi"),
            _ => None,
        }
    }
    // Immediate-only constraint (no register alternative)?
    fn imm_only(c: &str) -> bool {
        !c.is_empty() && c.chars().all(|ch| matches!(ch,
            'i' | 'n' | 'I' | 'J' | 'K' | 'L' | 'M' | 'N' | 'O' | 'e' | 'Z' | 's'))
    }

    let mut all_blocks_fit = true;
    'scan: for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::InlineAsm { outputs, inputs, clobbers, operand_types, .. } = inst
            else { continue };

            let mut avail: i32 = 2; // ecx, edx
            let mut ecx_gone = false;
            let mut edx_gone = false;
            let mut demand: i32 = 0;
            let mut op_idx = 0usize;

            let mut account = |c: &str, op_idx: usize,
                               ecx_gone: &mut bool, edx_gone: &mut bool,
                               avail: &mut i32, demand: &mut i32| {
                let c = c.trim_start_matches(['=', '+', '&', '%']);
                if c.starts_with("@cc") { return; }
                if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) { return; } // tied
                if imm_only(c) { return; }
                if let Some(reg) = specific_reg_letter(c) {
                    if reg == "ecx" && !*ecx_gone { *ecx_gone = true; *avail -= 1; }
                    if reg == "edx" && !*edx_gone { *edx_gone = true; *avail -= 1; }
                    return; // no scratch draw
                }
                let wide = matches!(operand_types.get(op_idx),
                    Some(IrType::I64) | Some(IrType::U64) | Some(IrType::F64));
                *demand += if wide { 2 } else { 1 };
            };

            for (c, _, _) in outputs {
                account(c, op_idx, &mut ecx_gone, &mut edx_gone, &mut avail, &mut demand);
                op_idx += 1;
            }
            for (c, _, _) in inputs {
                account(c, op_idx, &mut ecx_gone, &mut edx_gone, &mut avail, &mut demand);
                op_idx += 1;
            }
            for cl in clobbers {
                let name = cl.trim_start_matches('%');
                if (name == "ecx" || name == "cx") && !ecx_gone { ecx_gone = true; avail -= 1; }
                if (name == "edx" || name == "dx") && !edx_gone { edx_gone = true; avail -= 1; }
            }

            if demand > avail.max(0) {
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
        collect_inline_asm_callee_saved_inner(func, used, constraint_to_phys, clobber_to_phys, all_callee_saved);
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
            if let Instruction::InlineAsm { outputs, inputs, clobbers, .. } = inst {
                for (constraint, _, _) in outputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) { used.push(phys); }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        // Generic register class: conservatively mark all
                        // callee-saved registers as clobbered since the
                        // scratch allocator may pick any of them.
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) { used.push(phys); }
                        }
                    }
                }
                for (constraint, _, _) in inputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) { used.push(phys); }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) { used.push(phys); }
                        }
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) { used.push(phys); }
                    }
                }
            }
        }
    }
}
