//! Interprocedural Constant Propagation (IPCP).
//!
//! This pass performs three interprocedural optimizations:
//!
//! 1. **Constant return propagation**: Identifies static functions that always
//!    return the same constant value on every return path, and replaces calls
//!    to those functions with the constant.
//!
//! 2. **Dead call elimination**: Removes calls to side-effect-free void functions
//!    (empty stubs). This eliminates references to symbols that would otherwise
//!    cause linker errors (e.g., kernel's `apply_retpolines()` stub referencing
//!    `__retpoline_sites` when CONFIG_MITIGATION_RETPOLINE is disabled).
//!
//! 3. **Constant argument propagation**: When all call sites of a defined function
//!    pass the same constant for a given parameter, replaces the `ParamRef` in the
//!    function body with a `Copy` of that constant. Subsequent optimization passes
//!    (constant folding, DCE, CFG simplification) then eliminate dead code guarded
//!    by that parameter. This is critical for the Linux kernel where static functions
//!    like `__fpu_restore_sig` receive a parameter that is always false (due to
//!    `IS_ENABLED()` checks), and the false branch contains references to undefined
//!    symbols like `convert_to_fxsr`.
//!
//! This pass runs on every iteration of the optimization pipeline. Constants
//! at call sites may only become visible after earlier passes simplify phi
//! nodes and branches (e.g., from IS_ENABLED() checks). Subsequent constant
//! folding, DCE, and CFG simplification clean up the dead code.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrConst, IrModule, Operand, Terminator};

/// Run interprocedural constant propagation on the module.
///
/// Returns the number of changes made (call sites replaced, calls eliminated,
/// or parameters specialized with constants).
pub fn run(module: &mut IrModule) -> usize {
    let mut total_changes = 0;

    // Phase 0: bounded evaluation of direct constant calls. This is a
    // compile-time interpreter for pure integer IR, not a general symbolic
    // executor. It handles small recursive specializations such as
    // ackermann(3, 11) while refusing memory, inline asm, indirect calls,
    // unbounded loops, or calls with non-constant arguments.
    total_changes += evaluate_constant_calls(module);

    // Phase 1: Constant return propagation.
    // Find side-effect-free functions that always return the same constant,
    // and replace calls to them with the constant value.
    let const_returns = find_constant_return_functions(module);
    if !const_returns.is_empty() {
        for func in &mut module.functions {
            if func.is_declaration {
                continue;
            }
            for block in &mut func.blocks {
                let mut i = 0;
                while i < block.instructions.len() {
                    let replace = match &block.instructions[i] {
                        Instruction::Call { func: callee, info } => {
                            if let Some(dest) = info.dest {
                                const_returns
                                    .get(callee.as_str())
                                    .map(|const_val| (dest, *const_val))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some((dest, const_val)) = replace {
                        block.instructions[i] = Instruction::Copy {
                            dest,
                            src: Operand::Const(const_val),
                        };
                        total_changes += 1;
                    }
                    i += 1;
                }
            }
        }
    }

    // Phase 2: Dead call elimination for empty void functions.
    // Remove calls to functions whose body has no side effects and returns void.
    let dead_calls = find_dead_call_functions(module);
    if !dead_calls.is_empty() {
        for func in &mut module.functions {
            if func.is_declaration {
                continue;
            }
            for block in &mut func.blocks {
                let has_spans = !block.source_spans.is_empty();
                let mut new_insts = Vec::with_capacity(block.instructions.len());
                let mut new_spans = Vec::with_capacity(16);
                for (idx, inst) in block.instructions.drain(..).enumerate() {
                    let is_dead = match &inst {
                        Instruction::Call { func, .. } => dead_calls.contains(func.as_str()),
                        _ => false,
                    };
                    if !is_dead {
                        new_insts.push(inst);
                        if has_spans && idx < block.source_spans.len() {
                            new_spans.push(block.source_spans[idx]);
                        }
                    } else {
                        total_changes += 1;
                    }
                }
                block.instructions = new_insts;
                if has_spans {
                    block.source_spans = new_spans;
                }
            }
        }
    }

    // Phase 3: Constant argument propagation.
    // For each defined (non-weak) function, check if all call sites pass the same
    // constant for a given parameter. If so, replace the ParamRef with a Copy of
    // the constant, enabling subsequent passes to fold branches and eliminate dead code.
    total_changes += propagate_constant_arguments(module);

    total_changes
}

/// Evaluate direct calls whose arguments are all compile-time integer constants.
///
/// This intentionally works on a small, strict IR subset. Refusing a function
/// is the default: a single load, store, indirect call, volatile operation, or
/// unsupported loop shape makes evaluation return `None` and leaves the call
/// unchanged. The step budget and memo table bound compile-time work.
fn evaluate_constant_calls(module: &mut IrModule) -> usize {
    type EvalKey = (usize, Vec<crate::ir::constants::ConstHashKey>);
    let mut function_indices: FxHashMap<String, usize> = FxHashMap::default();
    for (idx, func) in module.functions.iter().enumerate() {
        if !func.is_declaration && func.is_static && !func.is_weak && !func.is_variadic {
            function_indices.insert(func.name.clone(), idx);
        }
    }
    if function_indices.is_empty() {
        return 0;
    }

    let mut replacements: Vec<(usize, usize, usize, IrConst)> = Vec::new();
    let mut memo: FxHashMap<EvalKey, Option<IrConst>> = FxHashMap::default();
    let mut active: FxHashSet<EvalKey> = FxHashSet::default();

    for (caller_idx, caller) in module.functions.iter().enumerate() {
        if caller.is_declaration {
            continue;
        }
        for (block_idx, block) in caller.blocks.iter().enumerate() {
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                let Instruction::Call { func: callee, info } = inst else {
                    continue;
                };
                let Some(dest) = info.dest else {
                    continue;
                };
                let Some(&callee_idx) = function_indices.get(callee) else {
                    continue;
                };
                if info
                    .args
                    .iter()
                    .any(|arg| !matches!(arg, Operand::Const(_)))
                {
                    continue;
                }
                let args: Vec<IrConst> = info
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        Operand::Const(c) => Some(*c),
                        Operand::Value(_) => None,
                    })
                    .collect();
                if args.len() != info.args.len() {
                    continue;
                }
                let mut budget = 2_000_000u64;
                if let Some(result) = eval_const_function(
                    module,
                    &function_indices,
                    callee_idx,
                    &args,
                    &mut memo,
                    &mut active,
                    &mut budget,
                ) {
                    replacements.push((caller_idx, block_idx, inst_idx, result));
                    if std::env::var("CCC_DEBUG_IPCP_EVAL").is_ok() {
                        eprintln!("[IPCP-EVAL] {} -> {} = {:?}", callee, dest.0, result);
                    }
                }
            }
        }
    }

    // Apply backwards within each block so instruction indices remain valid.
    replacements.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let mut changes = 0;
    for (caller_idx, block_idx, inst_idx, result) in replacements.into_iter().rev() {
        if let Some(Instruction::Call { info, .. }) = module
            .functions
            .get_mut(caller_idx)
            .and_then(|func| func.blocks.get_mut(block_idx))
            .and_then(|block| block.instructions.get(inst_idx))
        {
            if let Some(dest) = info.dest {
                module.functions[caller_idx].blocks[block_idx].instructions[inst_idx] =
                    Instruction::Copy {
                        dest,
                        src: Operand::Const(result),
                    };
                changes += 1;
            }
        }
    }
    changes
}

fn eval_const_function(
    module: &IrModule,
    function_indices: &FxHashMap<String, usize>,
    function_idx: usize,
    args: &[IrConst],
    memo: &mut FxHashMap<(usize, Vec<crate::ir::constants::ConstHashKey>), Option<IrConst>>,
    active: &mut FxHashSet<(usize, Vec<crate::ir::constants::ConstHashKey>)>,
    budget: &mut u64,
) -> Option<IrConst> {
    if *budget == 0 {
        return None;
    }
    let key = (function_idx, args.iter().map(|c| c.to_hash_key()).collect());
    if let Some(result) = memo.get(&key) {
        return *result;
    }
    if !active.insert(key.clone()) {
        return None;
    }
    *budget -= 1;

    let result = (|| {
        let func = module.functions.get(function_idx)?;
        if func.is_declaration
            || func.is_variadic
            || func.is_weak
            || func.params.len() != args.len()
        {
            return None;
        }
        let label_to_idx: FxHashMap<u32, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(idx, block)| (block.label.0, idx))
            .collect();
        let mut values: FxHashMap<u32, IrConst> = FxHashMap::default();
        let mut current_idx = 0usize;
        let mut predecessor: Option<u32> = None;

        loop {
            let block = func.blocks.get(current_idx)?;
            let mut first_non_phi = 0usize;
            while first_non_phi < block.instructions.len() {
                let Instruction::Phi { dest, incoming, .. } = &block.instructions[first_non_phi]
                else {
                    break;
                };
                let pred = predecessor?;
                let op = incoming
                    .iter()
                    .find(|(_, label)| label.0 == pred)
                    .map(|(op, _)| op)?;
                values.insert(dest.0, eval_const_operand(op, &values)?);
                first_non_phi += 1;
            }
            for inst in block.instructions.iter().skip(first_non_phi) {
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                match inst {
                    // Residual allocas are harmless when mem2reg has removed
                    // every load/store use; keep them as no-ops in the pure
                    // evaluator.
                    Instruction::Alloca { .. } => {}
                    Instruction::ParamRef {
                        dest, param_idx, ..
                    } => {
                        values.insert(dest.0, *args.get(*param_idx)?);
                    }
                    Instruction::Copy { dest, src } => {
                        values.insert(dest.0, eval_const_operand(src, &values)?);
                    }
                    // Cast/UnaryOp/BinOp/Cmp are evaluated by the canonical
                    // constant-fold evaluators so that the interprocedural
                    // interpreter can never disagree with the intraprocedural
                    // folder.  The previous hand-rolled arms went through
                    // `to_i64()` and ignored the operation TYPE: a `Cmp Ne` at
                    // U16 between `Cast U32(0xFFFFFFFF)->U16` (held as
                    // I64(65535)) and `I16(-1)` (the same 16-bit pattern)
                    // evaluated 65535 != -1 and proved a branch that was
                    // never taken, so a caller was folded onto its abort path.
                    Instruction::Cast {
                        dest,
                        src,
                        from_ty,
                        to_ty,
                    } => {
                        let value = eval_const_operand(src, &values)?;
                        values.insert(
                            dest.0,
                            crate::passes::constant_fold::eval_cast_const(
                                value, *from_ty, *to_ty,
                            )?,
                        );
                    }
                    Instruction::UnaryOp { dest, op, src, ty } => {
                        // `__builtin_constant_p` is phase-dependent; the pure
                        // evaluator must not commit to an answer for it.
                        if *op == crate::ir::reexports::IrUnaryOp::IsConstant {
                            return None;
                        }
                        let value = eval_const_operand(src, &values)?;
                        values.insert(
                            dest.0,
                            crate::passes::constant_fold::eval_unaryop_const(
                                *op, value, None, *ty,
                            )?,
                        );
                    }
                    Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let lhs = eval_const_operand(lhs, &values)?;
                        let rhs = eval_const_operand(rhs, &values)?;
                        values.insert(
                            dest.0,
                            crate::passes::constant_fold::eval_binop_const(*op, lhs, rhs, *ty)?,
                        );
                    }
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let lhs = eval_const_operand(lhs, &values)?;
                        let rhs = eval_const_operand(rhs, &values)?;
                        values.insert(
                            dest.0,
                            crate::passes::constant_fold::eval_cmp_const(*op, lhs, rhs, *ty)?,
                        );
                    }
                    Instruction::Select {
                        dest,
                        cond,
                        true_val,
                        false_val,
                        ..
                    } => {
                        let cond = eval_const_operand(cond, &values)?.to_i64()?;
                        let selected = if cond != 0 { true_val } else { false_val };
                        values.insert(dest.0, eval_const_operand(selected, &values)?);
                    }
                    Instruction::Call { func: callee, info } => {
                        let callee_idx = *function_indices.get(callee)?;
                        let mut call_args = Vec::with_capacity(info.args.len());
                        for arg in &info.args {
                            call_args.push(eval_const_operand(arg, &values)?);
                        }
                        let result = eval_const_function(
                            module,
                            function_indices,
                            callee_idx,
                            &call_args,
                            memo,
                            active,
                            budget,
                        )?;
                        if let Some(dest) = info.dest {
                            values.insert(dest.0, result);
                        } else {
                            return None;
                        }
                    }
                    // Memory, address-taking, indirect calls, atomics, inline
                    // asm, and target intrinsics are intentionally not evaluated.
                    _ => return None,
                }
            }

            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            match &block.terminator {
                Terminator::Return(Some(op)) => return eval_const_operand(op, &values),
                Terminator::Return(None) | Terminator::Unreachable => return None,
                Terminator::Branch(label) => {
                    predecessor = Some(block.label.0);
                    current_idx = *label_to_idx.get(&label.0)?;
                }
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    let cond = eval_const_operand(cond, &values)?.to_i64()?;
                    predecessor = Some(block.label.0);
                    let label = if cond != 0 { true_label } else { false_label };
                    current_idx = *label_to_idx.get(&label.0)?;
                }
                Terminator::Switch {
                    val,
                    cases,
                    default,
                    ..
                } => {
                    let value = eval_const_operand(val, &values)?.to_i64()?;
                    predecessor = Some(block.label.0);
                    let label = cases
                        .iter()
                        .find(|(case, _)| *case == value)
                        .map(|(_, label)| label)
                        .unwrap_or(default);
                    current_idx = *label_to_idx.get(&label.0)?;
                }
                Terminator::IndirectBranch { .. } => return None,
            }
        }
    })();

    active.remove(&key);
    memo.insert(key, result);
    result
}

fn eval_const_operand(op: &Operand, values: &FxHashMap<u32, IrConst>) -> Option<IrConst> {
    match op {
        Operand::Const(c) => Some(*c),
        Operand::Value(v) => values.get(&v.0).copied(),
    }
}

/// Analyze all static (internal-linkage) functions in the module and return
/// a map from function name to constant value for those that always return
/// the same constant on every path.
fn find_constant_return_functions(module: &IrModule) -> FxHashMap<String, IrConst> {
    let mut result = FxHashMap::default();

    for func in &module.functions {
        // Only analyze defined functions whose body we can see.
        // Both static and non-static functions are eligible: we're not removing
        // the function, just replacing calls within this TU with the constant.
        // Non-static (external linkage) functions still keep their definition
        // for other TUs to call. In C, having two strong definitions of the
        // same function is a linker error, so we can trust the body we see.
        if func.is_declaration {
            continue;
        }

        // Skip weak functions: they can be overridden by a strong definition
        // in another TU, so we can't trust the body we see.
        if func.is_weak {
            continue;
        }

        // Skip functions with no blocks (shouldn't happen for definitions)
        if func.blocks.is_empty() {
            continue;
        }

        // Skip variadic functions (they might have complex behavior)
        if func.is_variadic {
            continue;
        }

        // Check if the function body could have side effects.
        // We only want to replace calls to pure functions (no stores, no calls,
        // no inline asm, no atomics, etc.) that always return the same constant.
        if !is_side_effect_free(func) {
            continue;
        }

        // Collect all return values across all blocks
        let mut return_const: Option<IrConst> = None;
        let mut all_same = true;
        let mut has_return = false;

        for block in &func.blocks {
            if let Terminator::Return(Some(operand)) = &block.terminator {
                has_return = true;
                match operand {
                    Operand::Const(c) => {
                        if let Some(ref existing) = return_const {
                            if !const_equal(existing, c) {
                                all_same = false;
                                break;
                            }
                        } else {
                            return_const = Some(*c);
                        }
                    }
                    Operand::Value(_) => {
                        // Return value is computed, not a constant
                        all_same = false;
                        break;
                    }
                }
            } else if let Terminator::Return(None) = &block.terminator {
                // Void return - skip, we only care about value-returning functions
                has_return = true;
                all_same = false;
                break;
            }
            // Other terminators (Branch, CondBranch, Unreachable) don't affect this analysis
        }

        if has_return && all_same {
            if let Some(const_val) = return_const {
                result.insert(func.name.clone(), const_val);
            }
        }
    }

    result
}

/// Check if a function is pure (no observable side effects and result depends
/// only on inputs/constants). A pure function has no stores, no calls, no loads,
/// no inline asm, no atomics, etc. This is intentionally conservative: the target
/// use case is kernel config stubs that return literal constants without any
/// memory access.
fn is_side_effect_free(func: &crate::ir::reexports::IrFunction) -> bool {
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // These instructions have side effects:
                Instruction::PgoCounterInc { .. }
                | Instruction::Store { .. }
                | Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::SetStaticChain { .. }
                | Instruction::InitTrampoline { .. }
                | Instruction::NonlocalGotoSave { .. }
                | Instruction::NonlocalGoto { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicInc { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::Fence { .. }
                | Instruction::Memcpy { .. }
                | Instruction::VaStart { .. }
                | Instruction::VaEnd { .. }
                | Instruction::VaCopy { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::StackSave { .. }
                | Instruction::StackRestore { .. }
                | Instruction::Intrinsic { .. }
                | Instruction::VaArg { .. }
                | Instruction::VaArgStruct { .. }
                | Instruction::SetReturnF64Second { .. }
                | Instruction::SetReturnF32Second { .. }
                | Instruction::SetReturnF128Second { .. }
                | Instruction::Load { .. }
                | Instruction::AtomicLoad { .. } => {
                    // Loads read memory that could change between calls,
                    // so functions with loads aren't truly pure.
                    return false;
                }

                // These are pure (compute a value only from inputs/constants):
                Instruction::Alloca { .. }
                | Instruction::GetStaticChain { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::Phi { .. }
                | Instruction::Select { .. }
                | Instruction::LabelAddr { .. }
                | Instruction::ParamRef { .. } => {
                    // Pure: result depends only on operands, no memory access
                }

                // GetReturn* read implicit register state from a preceding Call,
                // but Call is already rejected above, so these are unreachable.
                // Classify as side-effecting for correctness if that ever changes.
                Instruction::GetReturnF64Second { .. }
                | Instruction::GetReturnF32Second { .. }
                | Instruction::GetReturnF128Second { .. } => {
                    return false;
                }
            }
        }
    }
    true
}

/// Compare two IR constants for equality using hash keys (consistent with GVN).
fn const_equal(a: &IrConst, b: &IrConst) -> bool {
    a.to_hash_key() == b.to_hash_key()
}

/// Find functions whose calls can be eliminated entirely.
///
/// A function qualifies if:
/// - It is defined (not a declaration)
/// - It is not weak (could be overridden)
/// - It is side-effect-free (no stores, calls, inline asm, etc.)
/// - It returns void (no return value to propagate)
///
/// Calls to such functions are dead: they do nothing observable and produce
/// no value. Eliminating them removes references to their arguments, which
/// may include undefined external symbols.
fn find_dead_call_functions(module: &IrModule) -> FxHashSet<String> {
    let mut result = FxHashSet::default();

    for func in &module.functions {
        if func.is_declaration || func.is_weak || func.blocks.is_empty() {
            continue;
        }
        // Must return void
        if func.return_type != crate::common::types::IrType::Void {
            continue;
        }
        // Must be side-effect-free
        if !is_side_effect_free(func) {
            continue;
        }
        // All terminators must be compatible with dead call elimination.
        // Unreachable terminators represent trap instructions (ud2/brk/ebreak)
        // which are observable side effects - functions containing them must
        // NOT be eliminated (e.g., functions wrapping __builtin_trap()).
        let safe_to_eliminate = func.blocks.iter().all(|b| {
            match &b.terminator {
                Terminator::Return(None) => true,
                Terminator::Return(Some(_)) => false,
                Terminator::Unreachable => false,
                // Non-return terminators (Branch, CondBranch, Switch, etc.) are fine
                _ => true,
            }
        });
        if !safe_to_eliminate {
            continue;
        }
        result.insert(func.name.clone());
    }

    result
}

/// Propagate constant arguments into function bodies.
///
/// For each defined function, collects all call sites across the module.
/// If every call site passes the same constant for a particular parameter,
/// replaces the `ParamRef` instruction for that parameter with a `Copy` of
/// the constant. This enables subsequent constant folding and DCE to
/// eliminate dead code guarded by that parameter.
///
/// This is critical for the Linux kernel where functions like `__fpu_restore_sig`
/// are too large to inline but always receive a constant argument (e.g.,
/// `ia32_fxstate = false` when CONFIG_IA32_EMULATION is disabled).
fn propagate_constant_arguments(module: &mut IrModule) -> usize {
    // Step 1: For each function name, collect the constant passed at each
    // parameter position across all call sites.
    // Maps function_name -> vec of per-param state.
    // ParamState::Unknown = no call sites seen yet
    // ParamState::Const(c) = all call sites pass constant c
    // ParamState::Varying = call sites pass different values
    let mut func_param_consts: FxHashMap<String, Vec<ParamState>> = FxHashMap::default();

    // First, identify candidate functions (static, defined, non-weak, non-variadic,
    // has ParamRef instructions). Only static functions are eligible because
    // non-static functions can be called from other translation units with
    // arbitrary argument values that we can't see.
    for func in &module.functions {
        if func.is_declaration || func.is_weak || func.blocks.is_empty() {
            continue;
        }
        if !func.is_static {
            continue;
        }
        if func.is_variadic {
            continue;
        }
        if func.params.is_empty() {
            continue;
        }
        // Check if the function has any ParamRef instructions
        let has_param_ref = func.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::ParamRef { .. }))
        });
        if !has_param_ref {
            continue;
        }
        func_param_consts.insert(
            func.name.clone(),
            vec![ParamState::Unknown; func.params.len()],
        );
    }

    if func_param_consts.is_empty() {
        return 0;
    }

    // Step 2: Scan all call sites and update per-param constant state.
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Call { func: callee, info } = inst {
                    if let Some(param_states) = func_param_consts.get_mut(callee.as_str()) {
                        for (i, arg) in info.args.iter().enumerate() {
                            if i >= param_states.len() {
                                break;
                            }
                            match arg {
                                Operand::Const(c) => match &param_states[i] {
                                    ParamState::Unknown => {
                                        param_states[i] = ParamState::Const(*c);
                                    }
                                    ParamState::Const(existing) => {
                                        if !const_equal(existing, c) {
                                            param_states[i] = ParamState::Varying;
                                        }
                                    }
                                    ParamState::Varying => {}
                                },
                                Operand::Value(_) => {
                                    param_states[i] = ParamState::Varying;
                                }
                            }
                        }
                    }
                }
                // Note: indirect calls (CallIndirect) are not scanned here, but safety
                // is ensured by the GlobalAddr check below -- any function whose address
                // is taken (prerequisite for indirect calls) has all its params marked Varying.
            }
        }
    }

    // Also check for address-taken functions: if a function's address is taken
    // (used in GlobalAddr), it could be called indirectly with unknown arguments.
    // Mark all its params as Varying.
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GlobalAddr { name, .. } = inst {
                    if let Some(param_states) = func_param_consts.get_mut(name.as_str()) {
                        for state in param_states.iter_mut() {
                            *state = ParamState::Varying;
                        }
                    }
                }
            }
        }
    }
    // Also check global initializers for address-taken references
    for global in &module.globals {
        global.init.for_each_ref(&mut |name| {
            if let Some(param_states) = func_param_consts.get_mut(name) {
                for state in param_states.iter_mut() {
                    *state = ParamState::Varying;
                }
            }
        });
    }

    // Step 3: Build a map of function_name -> vec of (param_idx, constant) for
    // parameters that have a uniform constant across all call sites.
    let mut specializations: FxHashMap<String, Vec<(usize, IrConst)>> = FxHashMap::default();
    for (name, param_states) in &func_param_consts {
        let mut specs = Vec::with_capacity(16);
        for (i, state) in param_states.iter().enumerate() {
            if let ParamState::Const(c) = state {
                specs.push((i, *c));
            }
        }
        if !specs.is_empty() {
            specializations.insert(name.clone(), specs);
        }
    }

    if specializations.is_empty() {
        return 0;
    }

    // Step 4: Apply specializations by replacing ParamRef with Copy of constant.
    let mut total = 0;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        if let Some(specs) = specializations.get(&func.name) {
            for block in &mut func.blocks {
                for inst in &mut block.instructions {
                    if let Instruction::ParamRef {
                        dest, param_idx, ..
                    } = inst
                    {
                        for (spec_idx, spec_const) in specs {
                            if *param_idx == *spec_idx {
                                *inst = Instruction::Copy {
                                    dest: *dest,
                                    src: Operand::Const(*spec_const),
                                };
                                total += 1;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    total
}

/// State of a parameter across all call sites.
#[derive(Clone, Debug)]
enum ParamState {
    /// No call sites observed yet.
    Unknown,
    /// All observed call sites pass this constant.
    Const(IrConst),
    /// Different call sites pass different values (or a non-constant).
    Varying,
}
