//! Interprocedural Purity and Constness Analysis.
//!
//! This pass infers whether functions are `pure` (read-only with respect to
//! non-local memory, no side effects) or `const` (no memory access at all
//! outside non-escaping locals, no side effects) and propagates these attributes
//! to all call sites in the module.
//!
//! GCC attributes:
//! - `__attribute__((__const__))`: function return value depends solely on its
//!   parameters; it does not read or write global memory and has no observable side effects.
//! - `__attribute__((__pure__))`: function return value depends only on its
//!   parameters and/or global memory; it does not write global memory and has no
//!   observable side effects.

use crate::common::fx_hash::FxHashSet;
use crate::common::types::AddressSpace;
use crate::ir::reexports::{CallInfo, Instruction, IrFunction, IrModule, Operand, Terminator};

/// Run interprocedural purity and constness inference on the module.
///
/// Returns the number of call sites updated.
pub fn run(module: &mut IrModule) -> usize {
    let mut pure_fns: FxHashSet<String> = FxHashSet::default();
    let mut const_fns: FxHashSet<String> = FxHashSet::default();

    // Standard math builtins known to be const
    for name in &[
        "abs", "labs", "llabs", "imaxabs", "fabs", "fabsf", "fabsl", "sqrt", "sqrtf", "sqrtl",
        "sin", "sinf", "sinl", "cos", "cosf", "cosl", "tan", "tanf", "tanl", "ceil", "ceilf",
        "ceill", "floor", "floorf", "floorl", "trunc", "truncf", "truncl", "round", "roundf",
        "roundl", "log", "logf", "logl", "exp", "expf", "expl", "pow", "powf", "powl",
    ] {
        const_fns.insert(name.to_string());
        pure_fns.insert(name.to_string());
    }

    // Standard string/mem builtins known to be pure
    for name in &[
        "strlen", "wcslen", "strcmp", "strncmp", "wcscmp", "wcsncmp", "memcmp", "bcmp", "strchr",
        "strrchr", "strstr", "memchr",
    ] {
        pure_fns.insert(name.to_string());
    }

    // Collect initial annotations from existing CallInfo in the module
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Call {
                    func: ref fname,
                    info,
                } = inst
                {
                    if info.is_const {
                        const_fns.insert(fname.clone());
                        pure_fns.insert(fname.clone());
                    } else if info.is_pure {
                        pure_fns.insert(fname.clone());
                    }
                }
            }
        }
    }

    // Iteratively infer const and pure status for defined functions
    loop {
        let mut progress = false;

        for func in &module.functions {
            // __weak definitions are not necessarily the linked definition
            // (a strong override replaces the symbol at link time), so their
            // bodies cannot license purity/constness for call sites: a
            // weak-empty void callee marked "pure" gets its calls dropped by
            // DCE even though the linked strong override has side effects
            // (kernel 6.18 vmemmap_set_pmd — vmemmap never mapped).
            if func.is_declaration || func.blocks.is_empty() || func.is_weak {
                continue;
            }

            let is_already_const = const_fns.contains(&func.name);
            let is_already_pure = pure_fns.contains(&func.name);

            if is_already_const {
                continue;
            }

            let (can_be_pure, can_be_const) = analyze_function_purity(func, &pure_fns, &const_fns);

            if can_be_const && !is_already_const {
                const_fns.insert(func.name.clone());
                pure_fns.insert(func.name.clone());
                progress = true;
            } else if can_be_pure && !is_already_pure {
                pure_fns.insert(func.name.clone());
                progress = true;
            }
        }

        if !progress {
            break;
        }
    }

    // Propagate inferred purity to all call sites
    let mut total_updates = 0;
    for func in &mut module.functions {
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                if let Instruction::Call {
                    func: ref callee,
                    ref mut info,
                } = inst
                {
                    if const_fns.contains(callee.as_str()) {
                        if !info.is_const {
                            info.is_const = true;
                            info.is_pure = true;
                            total_updates += 1;
                        }
                    } else if pure_fns.contains(callee.as_str()) {
                        if !info.is_pure {
                            info.is_pure = true;
                            total_updates += 1;
                        }
                    }
                }
            }
        }
    }

    total_updates
}

/// Analyze a function body to determine if it meets the criteria for `pure` and/or `const`.
fn analyze_function_purity(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
) -> (bool, bool) {
    // Collect local alloca destinations
    let mut local_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                if !volatile && !semantic_volatile {
                    local_allocas.insert(dest.0);
                }
            }
        }
    }

    // Determine escaping allocas (e.g. passed as call arguments or stored)
    let mut escaped_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { .. } | Instruction::Copy { .. } => {}
                Instruction::Store { val, .. } => {
                    if let Operand::Value(v) = val {
                        if local_allocas.contains(&v.0) {
                            escaped_allocas.insert(v.0);
                        }
                    }
                }
                Instruction::Load { .. } => {}
                Instruction::Memcpy { dest, src, .. } => {
                    if !local_allocas.contains(&dest.0) {
                        escaped_allocas.insert(dest.0);
                    }
                    if !local_allocas.contains(&src.0) {
                        escaped_allocas.insert(src.0);
                    }
                }
                _ => {
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if local_allocas.contains(&v.0) {
                                escaped_allocas.insert(v.0);
                            }
                        }
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if local_allocas.contains(&v.0) {
                    escaped_allocas.insert(v.0);
                }
            }
        });
    }

    let mut is_pure = true;
    let mut is_const = true;

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Store {
                    ptr,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *volatile || *seg_override != AddressSpace::Default {
                        return (false, false);
                    }
                    // Writing to a non-escaping local alloca is allowed for pure/const functions.
                    // Writing to any other pointer invalidates purity and constness.
                    if !local_allocas.contains(&ptr.0) || escaped_allocas.contains(&ptr.0) {
                        return (false, false);
                    }
                }
                Instruction::Load {
                    ptr,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *volatile || *seg_override != AddressSpace::Default {
                        return (false, false);
                    }
                    // Reading from non-local memory (globals, params, heap) is allowed for `pure`
                    // but not for `const`.
                    if !local_allocas.contains(&ptr.0) || escaped_allocas.contains(&ptr.0) {
                        is_const = false;
                    }
                }
                Instruction::Call { func: callee, info } => {
                    let callee_const = const_fns.contains(callee.as_str()) || info.is_const;
                    let callee_pure = pure_fns.contains(callee.as_str()) || info.is_pure;

                    if !callee_const {
                        is_const = false;
                    }
                    if !callee_pure && !callee_const {
                        return (false, false);
                    }
                }
                Instruction::CallIndirect { info, .. } => {
                    if !info.is_const {
                        is_const = false;
                    }
                    if !info.is_pure && !info.is_const {
                        return (false, false);
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    if !local_allocas.contains(&dest.0) || escaped_allocas.contains(&dest.0) {
                        return (false, false);
                    }
                    if !local_allocas.contains(&src.0) || escaped_allocas.contains(&src.0) {
                        is_const = false;
                    }
                }
                Instruction::DynAlloca { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicInc { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::VaStart { .. }
                | Instruction::VaEnd { .. }
                | Instruction::VaCopy { .. }
                | Instruction::VaArg { .. }
                | Instruction::VaArgStruct { .. }
                | Instruction::PgoCounterInc { .. }
                | Instruction::Fence { .. }
                | Instruction::InitTrampoline { .. }
                | Instruction::NonlocalGotoSave { .. }
                | Instruction::NonlocalGoto { .. }
                | Instruction::StackRestore { .. } => {
                    return (false, false);
                }
                // Non-local control flow intrinsics: __builtin_setjmp returns
                // twice and __builtin_longjmp never returns.  Classifying them
                // as purity-neutral let DCE delete the very calls that perform
                // the longjmp (gcc.c-torture execute/pr60003.c at -O1/-O2,
                // where foo's `while (1) { a = 1; bar (); }` loop collapsed to
                // an empty infinite loop).  They must be treated exactly like
                // NonlocalGoto: neither pure nor const.
                Instruction::Intrinsic { op, .. }
                    if matches!(
                        op,
                        crate::ir::intrinsics::IntrinsicOp::BuiltinSetjmp
                            | crate::ir::intrinsics::IntrinsicOp::BuiltinLongjmp
                            | crate::ir::intrinsics::IntrinsicOp::SaveApplyArgs
                            | crate::ir::intrinsics::IntrinsicOp::DoBuiltinApply
                            | crate::ir::intrinsics::IntrinsicOp::RestoreApplyResult
                    ) =>
                {
                    return (false, false);
                }
                // Pure value computations / addressing
                Instruction::Alloca { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::Phi { .. }
                | Instruction::Select { .. }
                | Instruction::ParamRef { .. }
                | Instruction::StackSave { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::LabelAddr { .. }
                | Instruction::GetReturnF64Second { .. }
                | Instruction::SetReturnF64Second { .. }
                | Instruction::GetReturnF32Second { .. }
                | Instruction::SetReturnF32Second { .. }
                | Instruction::GetReturnF128Second { .. }
                | Instruction::SetReturnF128Second { .. }
                | Instruction::GetStaticChain { .. }
                | Instruction::SetStaticChain { .. }
                | Instruction::Intrinsic { .. } => {}
            }
        }

        match &block.terminator {
            Terminator::Return(_)
            | Terminator::Branch(_)
            | Terminator::CondBranch { .. }
            | Terminator::Switch { .. }
            | Terminator::Unreachable => {}
            Terminator::IndirectBranch { .. } => {
                return (false, false);
            }
        }
    }

    (is_pure, is_const)
}
