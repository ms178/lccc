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

/// The registers the i686 scratch allocator hands an inline asm, in its order
/// of preference (`assign_scratch_reg` in `i686/codegen/asm_emitter.rs`).
///
/// One definition for the allocator and for the two functions below that
/// predict it: which callee-saved register the prologue has to save depends
/// on how far down this list a block can reach.
pub const I686_ASM_SCRATCH_POOL: &[&str] = &["ecx", "edx", "esi", "edi", "eax", "ebx"];

/// The same for an operand printed through a byte view: IA-32 has no byte
/// name for `esi`, `edi` or `ebp`.
pub const I686_ASM_BYTE_SCRATCH_POOL: &[&str] = &["ecx", "edx", "eax", "ebx"];

/// What one operand of an i686 inline asm asks of the scratch pool.
enum I686ScratchDraw {
    /// Nothing: a tied operand, an immediate, a flag written straight to its
    /// byte home.
    Nothing,
    /// A register of its own, named by a single-register constraint letter.
    /// The scratch allocator passes over it for the other operands.
    Named(&'static str),
    /// This many registers of the pool, at most.
    Registers(i32),
}

fn i686_operand_scratch_draw(
    constraint: &str,
    op_idx: usize,
    outputs: &[(String, crate::ir::reexports::Value, Option<String>)],
    operand_types: &[crate::common::types::IrType],
    alloca_ids: &FxHashSet<u32>,
) -> I686ScratchDraw {
    use crate::common::types::IrType;
    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
    if c.starts_with("@cc") {
        let direct_byte_store = matches!(operand_types.get(op_idx), Some(IrType::I8 | IrType::U8))
            && outputs
                .get(op_idx)
                .is_some_and(|(_, ptr, _)| alloca_ids.contains(&ptr.0));
        return if direct_byte_store {
            I686ScratchDraw::Nothing
        } else {
            I686ScratchDraw::Registers(1)
        };
    }
    // Tied to another operand.
    if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) {
        return I686ScratchDraw::Nothing;
    }
    // Immediate-only: no register alternative.
    let imm_only = !c.is_empty()
        && c.chars().all(|ch| {
            matches!(
                ch,
                'i' | 'n' | 'I' | 'J' | 'K' | 'L' | 'M' | 'N' | 'O' | 'e' | 'Z' | 's'
            )
        });
    if imm_only {
        return I686ScratchDraw::Nothing;
    }
    let named = match c {
        "a" => Some("eax"),
        "b" => Some("ebx"),
        "c" => Some("ecx"),
        "d" => Some("edx"),
        "S" => Some("esi"),
        "D" => Some("edi"),
        _ => None,
    };
    if let Some(reg) = named {
        return I686ScratchDraw::Named(reg);
    }
    // i686 holds a 64-bit operand in a register pair.
    let wide = matches!(
        operand_types.get(op_idx),
        Some(IrType::I64) | Some(IrType::U64) | Some(IrType::F64)
    );
    I686ScratchDraw::Registers(if wide { 2 } else { 1 })
}

/// The pool register a clobber names, through any of its views (`sil`,
/// `dil` and the 64-bit names GCC takes in 32-bit code included: they were
/// not, so the walk below handed out a register the statement destroys).
fn i686_clobbered_pool_reg(clobber: &str) -> Option<&'static str> {
    crate::backend::inline_asm::x86_gpr32_of_name(clobber)
        .filter(|reg| I686_ASM_SCRATCH_POOL.contains(reg))
}

/// The callee-saved registers the i686 scratch allocator can hand to an
/// inline asm, now that the register allocator has chosen its homes.
///
/// [`collect_inline_asm_callee_saved_i686`] runs ahead of the allocator and
/// proves that a block is served from the head of the pool, `%ecx` and
/// `%edx`.  The scratch allocator, though, passes over a register that is
/// some value's home -- taking it would destroy the value -- and the
/// allocator is free to make homes of `%ecx` and `%edx`.  With both taken
/// the pool begins further down, at a callee-saved register nothing saved:
///
/// ```text
/// f:  pushl %ebx
///     pushl %esi                  ; the homes the allocator used
///     ...
///     movl 32(%esp), %edi         ; "r" operand: ecx, edx, esi are homes
///     #APP  testl %edi, %edi  #NO_APP
///     ...
///     popl %esi
///     popl %ebx
///     ret                         ; the caller's %edi is gone
/// ```
///
/// So the walk is repeated here with the homes known: for every block, the
/// registers it can draw are the first `demand` entries of the pool that are
/// neither a home nor named by the block itself, and the callee-saved ones
/// among them are returned for the prologue to save.  A block that may print
/// an operand through a byte view is walked through the byte pool as well.
/// The answer is a superset of what the allocator hands out (the demand is
/// the same over-approximation the early proof uses), and it is empty
/// whenever a caller-saved register is left at the head of the pool, which
/// is what the early proof assumed.
pub fn i686_asm_scratch_callee_saved(
    func: &IrFunction,
    is_home: impl Fn(&str) -> bool,
    ebx_reserved: bool,
) -> Vec<&'static str> {
    use crate::common::types::IrType;
    let mut alloca_ids: Option<FxHashSet<u32>> = None;
    let mut saved: Vec<&'static str> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::InlineAsm {
                template,
                outputs,
                inputs,
                clobbers,
                operand_types,
                ..
            } = inst
            else {
                continue;
            };
            let allocas = alloca_ids.get_or_insert_with(|| {
                func.blocks
                    .iter()
                    .flat_map(|block| block.instructions.iter())
                    .filter_map(|inst| match inst {
                        Instruction::Alloca { dest, .. } => Some(dest.0),
                        _ => None,
                    })
                    .collect()
            });
            let mut demand = 0usize;
            let mut named: Vec<&'static str> = Vec::new();
            let constraints = outputs
                .iter()
                .map(|(c, _, _)| c)
                .chain(inputs.iter().map(|(c, _, _)| c));
            for (op_idx, constraint) in constraints.enumerate() {
                match i686_operand_scratch_draw(constraint, op_idx, outputs, operand_types, allocas)
                {
                    I686ScratchDraw::Nothing => {}
                    I686ScratchDraw::Named(reg) => named.push(reg),
                    I686ScratchDraw::Registers(n) => demand += n as usize,
                }
            }
            if demand == 0 {
                continue;
            }
            named.extend(clobbers.iter().filter_map(|c| i686_clobbered_pool_reg(c)));
            let byte_view = template.contains("%b")
                || template.contains("%h")
                || operand_types
                    .iter()
                    .any(|ty| matches!(ty, IrType::I8 | IrType::U8))
                || outputs.iter().any(|(c, _, _)| {
                    c.trim_start_matches(['=', '+', '&', '%'])
                        .starts_with("@cc")
                });
            let pools: &[&[&'static str]] = if byte_view {
                &[I686_ASM_SCRATCH_POOL, I686_ASM_BYTE_SCRATCH_POOL]
            } else {
                &[I686_ASM_SCRATCH_POOL]
            };
            for pool in pools {
                let drawn = pool
                    .iter()
                    .copied()
                    .filter(|&reg| {
                        !(ebx_reserved && reg == "ebx") && !named.contains(&reg) && !is_home(reg)
                    })
                    .take(demand);
                for reg in drawn {
                    if matches!(reg, "ebx" | "esi" | "edi") && !saved.contains(&reg) {
                        saved.push(reg);
                    }
                }
            }
        }
    }
    saved
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

            let constraints = outputs
                .iter()
                .map(|(c, _, _)| c)
                .chain(inputs.iter().map(|(c, _, _)| c));
            for constraint in constraints {
                match i686_operand_scratch_draw(
                    constraint,
                    op_idx,
                    outputs,
                    operand_types,
                    &alloca_ids,
                ) {
                    I686ScratchDraw::Nothing => {}
                    I686ScratchDraw::Named(reg) => {
                        if reg == "ecx" && !ecx_gone {
                            ecx_gone = true;
                            avail -= 1;
                        }
                        if reg == "edx" && !edx_gone {
                            edx_gone = true;
                            avail -= 1;
                        }
                    }
                    I686ScratchDraw::Registers(n) => demand += n,
                }
                op_idx += 1;
            }
            for cl in clobbers {
                let name = i686_clobbered_pool_reg(cl);
                if name == Some("ecx") && !ecx_gone {
                    ecx_gone = true;
                    avail -= 1;
                }
                if name == Some("edx") && !edx_gone {
                    edx_gone = true;
                    avail -= 1;
                }
            }

            // A flag output written straight to its byte stack home
            // (`setcc <slot>`, the `direct_byte_store` case above) consumes no
            // scratch register, so it cannot perturb the caller-saved prefix
            // the mixed-shape rule below guards. Counting it as if it did
            // reserved the entire callee-saved pool -- a push/pop in the
            // prologue -- for instructions that never needed one.
            let mut has_scratch_cc_output = false;
            for (op_idx, (constraint, ptr, _)) in outputs.iter().enumerate() {
                let stripped = constraint.trim_start_matches(['=', '+', '&', '%']);
                let direct_byte_store =
                    matches!(operand_types.get(op_idx), Some(IrType::I8 | IrType::U8))
                        && alloca_ids.contains(&ptr.0);
                if stripped.starts_with("@cc") && !direct_byte_store {
                    has_scratch_cc_output = true;
                    break;
                }
            }
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
                || (has_scratch_cc_output && has_register_output && has_register_input)
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
            source_inline_ids: Vec::new(),
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

    /// An asm with `inputs` as its register operands, typed as given.
    fn asm_with_inputs(template: &str, inputs: &[(&str, IrType)], clobbers: &[&str]) -> IrFunction {
        make_func_with_asm(Instruction::InlineAsm {
            template: template.to_string(),
            outputs: vec![],
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, (c, _))| (c.to_string(), Operand::Value(Value(i as u32)), None))
                .collect(),
            clobbers: clobbers.iter().map(|c| c.to_string()).collect(),
            operand_types: inputs.iter().map(|(_, ty)| *ty).collect(),
            goto_labels: vec![],
            input_symbols: vec![None; inputs.len()],
            seg_overrides: vec![AddressSpace::Default; inputs.len()],
        })
    }

    fn scratch_saved(func: &IrFunction, homes: &[&str], ebx_reserved: bool) -> Vec<&'static str> {
        i686_asm_scratch_callee_saved(func, |reg| homes.contains(&reg), ebx_reserved)
    }

    #[test]
    fn i686_scratch_stays_caller_saved_while_ecx_or_edx_is_no_home() {
        let one = asm_with_inputs("testl %0, %0", &[("r", IrType::I32)], &[]);
        assert!(scratch_saved(&one, &[], false).is_empty());
        assert!(scratch_saved(&one, &["ecx"], false).is_empty());
        assert!(scratch_saved(&one, &["edx", "esi", "edi", "ebx"], false).is_empty());
    }

    #[test]
    fn i686_scratch_reaches_a_callee_saved_register_past_two_caller_saved_homes() {
        // The shape that lost the caller's %edi: %ecx, %edx and %esi are homes,
        // so the first register of the pool that is free is %edi.
        let one = asm_with_inputs("testl %0, %0", &[("r", IrType::I32)], &[]);
        assert_eq!(scratch_saved(&one, &["ecx", "edx"], false), ["esi"]);
        assert_eq!(scratch_saved(&one, &["ecx", "edx", "esi"], false), ["edi"]);
        // %eax stands ahead of %ebx in the pool and is caller-saved.
        assert!(scratch_saved(&one, &["ecx", "edx", "esi", "edi"], false).is_empty());
        assert_eq!(
            scratch_saved(&one, &["ecx", "edx", "esi", "edi", "eax"], false),
            ["ebx"]
        );
        // The GOT register is never handed out, so there is nothing to save.
        assert!(scratch_saved(&one, &["ecx", "edx", "esi", "edi", "eax"], true).is_empty());
    }

    #[test]
    fn i686_scratch_counts_every_register_a_block_can_draw() {
        let two = asm_with_inputs(
            "addl %0, %1",
            &[("r", IrType::I32), ("r", IrType::I32)],
            &[],
        );
        assert!(scratch_saved(&two, &[], false).is_empty());
        assert_eq!(scratch_saved(&two, &["ecx"], false), ["esi"]);
        assert_eq!(scratch_saved(&two, &["ecx", "edx"], false), ["esi", "edi"]);
        // A 64-bit operand is a register pair.
        let wide = asm_with_inputs("nop", &[("r", IrType::I64)], &[]);
        assert!(scratch_saved(&wide, &[], false).is_empty());
        assert_eq!(scratch_saved(&wide, &["edx"], false), ["esi"]);
    }

    #[test]
    fn i686_scratch_passes_over_registers_the_block_names() {
        // "c" takes %ecx for itself and a clobber takes %edx: the one generic
        // operand starts at %esi without any home at all.
        let named = asm_with_inputs(
            "shll %b0, %1",
            &[("c", IrType::I32), ("r", IrType::I32)],
            &["edx"],
        );
        assert_eq!(scratch_saved(&named, &[], false), ["esi"]);
        // Views of a register name it too.
        let sub = asm_with_inputs("nop", &[("r", IrType::I32)], &["cl", "dx"]);
        assert_eq!(scratch_saved(&sub, &[], false), ["esi"]);
    }

    #[test]
    fn i686_byte_operands_are_walked_through_the_byte_pool() {
        // A byte operand cannot live in %esi or %edi: with %ecx, %edx and %eax
        // homes its register is %ebx.
        let byte = asm_with_inputs("testb %0, %0", &[("q", IrType::I8)], &[]);
        assert_eq!(
            scratch_saved(&byte, &["ecx", "edx", "eax"], false),
            ["esi", "ebx"]
        );
        // The same for a `%b` view of a wider operand.
        let view = asm_with_inputs("testb %b0, %b0", &[("r", IrType::I32)], &[]);
        assert_eq!(
            scratch_saved(&view, &["ecx", "edx", "eax"], false),
            ["esi", "ebx"]
        );
        // Without a byte view only the general pool is walked.
        let full = asm_with_inputs("testl %0, %0", &[("r", IrType::I32)], &[]);
        assert_eq!(scratch_saved(&full, &["ecx", "edx", "eax"], false), ["esi"]);
    }

    #[test]
    fn i686_scratch_needs_nothing_for_an_asm_that_draws_no_register() {
        let homes = ["ecx", "edx", "esi", "edi", "eax"];
        let bare = asm_with_inputs("cli", &[], &[]);
        assert!(scratch_saved(&bare, &homes, false).is_empty());
        let fixed = asm_with_inputs(
            "outb %b0, %w1",
            &[("a", IrType::I8), ("d", IrType::I16), ("i", IrType::I32)],
            &[],
        );
        assert!(scratch_saved(&fixed, &homes, false).is_empty());
    }

    #[test]
    fn i686_early_proof_and_late_walk_share_one_pool() {
        // The early proof reads the head of the pool as the two caller-saved
        // registers; the late walk reads the rest.  Both hold only as long as
        // the allocator's list is this one.
        assert_eq!(&I686_ASM_SCRATCH_POOL[..2], ["ecx", "edx"]);
        assert_eq!(&I686_ASM_BYTE_SCRATCH_POOL[..2], ["ecx", "edx"]);
        assert!(
            I686_ASM_BYTE_SCRATCH_POOL
                .iter()
                .all(|reg| I686_ASM_SCRATCH_POOL.contains(reg))
        );
    }
}
