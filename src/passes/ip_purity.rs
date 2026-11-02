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

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    BlockId, CallInfo, Instruction, IrFunction, IrModule, Operand, Terminator,
};

/// Run interprocedural purity and constness inference on the module.
///
/// Returns the number of call sites updated.
pub fn run(module: &mut IrModule) -> usize {
    run_with_builtins(
        module,
        &crate::common::pass_policy::BuiltinPolicy::default(),
    )
}

pub fn run_with_builtins(
    module: &mut IrModule,
    builtins: &crate::common::pass_policy::BuiltinPolicy,
) -> usize {
    let fenv = module
        .functions
        .iter()
        .any(|f| f.pass_policy.fp_environment_observable);
    let fp_effectful: FxHashSet<String> = module
        .functions
        .iter()
        .filter(|f| super::fp_environment_blocks_reordering(f))
        .map(|f| f.name.clone())
        .collect();
    let mut total_updates = 0;
    // Invalidate prior/declared attributes when FP environment effects are
    // possible. This runs before importing call annotations into the fixpoint.
    for func in &mut module.functions {
        if !func.pass_policy.fp_environment_observable {
            continue;
        }
        for inst in func.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
            let blocked = match &*inst {
                Instruction::Call { func, info } => {
                    fp_effectful.contains(func)
                        || info.return_type.is_float()
                        || info.arg_types.iter().any(|ty| ty.is_float())
                }
                Instruction::CallIndirect { .. } => true,
                _ => false,
            };
            if blocked {
                if let Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } =
                    inst
                {
                    total_updates += usize::from(info.is_pure || info.is_const);
                    info.is_pure = false;
                    info.is_const = false;
                }
            }
        }
    }
    let mut pure_fns: FxHashSet<String> = FxHashSet::default();
    let mut const_fns: FxHashSet<String> = FxHashSet::default();
    // What the C library guarantees about its own functions.  Kept apart
    // from the inferred/annotated sets: it describes the LIBRARY function,
    // so it applies only to calls known to reach it
    // (`CallInfo::assume_library` -- false under `-fno-builtin[-<fn>]` /
    // `-ffreestanding`), never to a name.
    let mut lib = LibraryFacts::default();

    // Standard math builtins known to be const
    for name in &[
        "abs", "labs", "llabs", "imaxabs", "fabs", "fabsf", "fabsl", "sqrt", "sqrtf", "sqrtl",
        "sin", "sinf", "sinl", "cos", "cosf", "cosl", "tan", "tanf", "tanl", "ceil", "ceilf",
        "ceill", "floor", "floorf", "floorl", "trunc", "truncf", "truncl", "round", "roundf",
        "roundl", "log", "logf", "logl", "exp", "expf", "expl", "pow", "powf", "powl",
    ] {
        if !builtins.may_assume(name)
            || fp_effectful.contains(*name)
            || (fenv && !["abs", "labs", "llabs", "imaxabs"].contains(name))
        {
            continue;
        }
        lib.const_fns.insert(*name);
    }

    // Standard string/mem builtins known to be pure
    for name in &[
        "strlen", "wcslen", "strcmp", "strncmp", "wcscmp", "wcsncmp", "memcmp", "bcmp", "strchr",
        "strrchr", "strstr", "memchr",
    ] {
        if !fp_effectful.contains(*name) {
            lib.pure_fns.insert(*name);
        }
    }

    // Collect initial annotations from existing CallInfo in the module
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Call { func: fname, info } = inst {
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

    // Iteratively infer const, pure, and non-capturing out-param summaries for defined functions
    let mut out_param_fns: FxHashMap<String, OutParamSummary> = FxHashMap::default();
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

            let (can_be_pure, can_be_const) = analyze_function_purity_with_summaries(
                func,
                &pure_fns,
                &const_fns,
                &out_param_fns,
                &lib,
            );

            if can_be_const && !is_already_const {
                const_fns.insert(func.name.clone());
                pure_fns.insert(func.name.clone());
                progress = true;
            } else if can_be_pure && !is_already_pure {
                pure_fns.insert(func.name.clone());
                progress = true;
            } else if !is_already_pure
                && func.is_static
                && let Some(summary) =
                    analyze_out_param_summary(func, &pure_fns, &const_fns, &out_param_fns, &lib)
                && out_param_fns.get(&func.name) != Some(&summary)
            {
                out_param_fns.insert(func.name.clone(), summary);
                progress = true;
            }
        }

        if !progress {
            break;
        }
    }

    // Propagate inferred purity to all call sites
    for func in &mut module.functions {
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                if let &mut Instruction::Call {
                    func: ref callee,
                    ref mut info,
                } = inst
                {
                    if const_fns.contains(callee.as_str()) || lib.is_const(callee, info) {
                        if !info.is_const {
                            info.is_const = true;
                            info.is_pure = true;
                            total_updates += 1;
                        }
                    } else if pure_fns.contains(callee.as_str()) || lib.is_pure(callee, info) {
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

/// Purity the C library guarantees for its own functions (a const function
/// is also pure).
#[derive(Default)]
struct LibraryFacts {
    const_fns: FxHashSet<&'static str>,
    pure_fns: FxHashSet<&'static str>,
}

impl LibraryFacts {
    fn is_const(&self, callee: &str, info: &CallInfo) -> bool {
        info.assume_library && self.const_fns.contains(callee)
    }

    fn is_pure(&self, callee: &str, info: &CallInfo) -> bool {
        info.assume_library && (self.pure_fns.contains(callee) || self.const_fns.contains(callee))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OutParamSummary {
    /// Bitmask of pointer parameter indices (< 64) written through by the callee.
    out_param_mask: u64,
    /// Bitmask of pointer parameter indices (< 64) whose pointer values may be
    /// stored into one of the callee's `out_param_mask` objects (e.g. a parser
    /// writing `cell->z = body` into a caller-owned `Cell` struct).
    captured_into_out_mask: u64,
    /// True if the callee may read memory outside its own non-escaping locals.
    reads_nonlocal: bool,
}

/// Successor block indices of `u`, restricted to blocks that exist in this
/// function (a `Switch` case or `IndirectBranch` target may name a block that
/// was pruned).
fn block_successors(
    func: &IrFunction,
    u: usize,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> Vec<usize> {
    let mut targets: Vec<BlockId> = Vec::new();
    match &func.blocks[u].terminator {
        Terminator::Branch(t) => targets.push(*t),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            targets.push(*true_label);
            targets.push(*false_label);
        }
        Terminator::Switch { cases, default, .. } => {
            targets.push(*default);
            for &(_, lbl) in cases {
                targets.push(lbl);
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => targets.extend(possible_targets.iter().copied()),
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
    targets
        .into_iter()
        .filter_map(|lbl| label_to_idx.get(&lbl).copied())
        .collect()
}

/// True when the control-flow graph reachable from block 0 contains a cycle.
///
/// Iterative three-colour DFS over a precomputed successor table: recursion
/// depth here would be proportional to the block count, and a large machine
/// generated function (or a switch-dispatch state machine) would overflow the
/// compiler's own stack on a valid input. The successor table is built once, so
/// each edge is materialised and walked exactly once.
fn func_has_back_edge(func: &IrFunction) -> bool {
    if func.blocks.is_empty() {
        return false;
    }
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let n = func.blocks.len();
    let succ: Vec<Vec<usize>> = (0..n)
        .map(|u| block_successors(func, u, &label_to_idx))
        .collect();
    // 0 = white (unvisited), 1 = gray (on the DFS path), 2 = black (finished).
    let mut color = vec![0u8; n];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..n {
        if color[start] != 0 {
            continue;
        }
        color[start] = 1;
        stack.push((start, 0));
        // Each iteration copies the frame out of the stack before touching it,
        // so the `push` below never overlaps a live `&mut` into the vector.
        while let Some(&(u, next)) = stack.last() {
            if next >= succ[u].len() {
                color[u] = 2;
                stack.pop();
                continue;
            }
            let v = succ[u][next];
            if let Some(top) = stack.last_mut() {
                top.1 = next + 1;
            }
            match color[v] {
                1 => return true,
                0 => {
                    color[v] = 1;
                    stack.push((v, 0));
                }
                _ => {}
            }
        }
    }
    false
}

fn collect_single_def_counts(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut counts: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                *counts.entry(d.0).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn collect_local_alloca_roots(
    func: &IrFunction,
    def_counts: &FxHashMap<u32, u32>,
) -> FxHashMap<u32, u32> {
    let mut ptr_to_root: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                ..
            } = inst
                && !volatile
                && !semantic_volatile
                && def_counts.get(&dest.0) == Some(&1)
            {
                ptr_to_root.insert(dest.0, dest.0);
            }
        }
    }
    if ptr_to_root.is_empty() {
        return ptr_to_root;
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    }
                    | Instruction::GetElementPtr {
                        dest, base: src, ..
                    } if def_counts.get(&dest.0) == Some(&1) => {
                        if let Some(&root) = ptr_to_root.get(&src.0)
                            && ptr_to_root.insert(dest.0, root) != Some(root)
                        {
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if !changed {
            break;
        }
    }
    ptr_to_root
}

fn find_escaped_alloca_roots(
    func: &IrFunction,
    ptr_to_root: &FxHashMap<u32, u32>,
    out_param_fns: &FxHashMap<String, OutParamSummary>,
) -> FxHashSet<u32> {
    let mut escaped: FxHashSet<u32> = FxHashSet::default();
    if ptr_to_root.is_empty() {
        return escaped;
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { .. } | Instruction::DynAlloca { .. } => {}
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => {
                    if let Some(&root) = ptr_to_root.get(&src.0)
                        && !ptr_to_root.contains_key(&dest.0)
                    {
                        escaped.insert(root);
                    }
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    if let Some(&root) = ptr_to_root.get(&base.0)
                        && !ptr_to_root.contains_key(&dest.0)
                    {
                        escaped.insert(root);
                    }
                    if let Operand::Value(off) = offset
                        && let Some(&root) = ptr_to_root.get(&off.0)
                    {
                        escaped.insert(root);
                    }
                }
                Instruction::Load { .. } => {}
                Instruction::Store { val, .. } => {
                    if let Operand::Value(v) = val
                        && let Some(&root) = ptr_to_root.get(&v.0)
                    {
                        escaped.insert(root);
                    }
                }
                Instruction::Memcpy { .. } => {}
                Instruction::Call { func: callee, info }
                    if !info.is_sret
                        && !info.is_variadic
                        && out_param_fns.contains_key(callee.as_str()) =>
                {
                    let summary = out_param_fns[callee.as_str()];
                    if summary.captured_into_out_mask != 0 {
                        for (idx, arg) in info.args.iter().enumerate() {
                            if idx < 64
                                && ((summary.captured_into_out_mask >> idx) & 1) != 0
                                && let Operand::Value(v) = arg
                                && let Some(&root) = ptr_to_root.get(&v.0)
                            {
                                escaped.insert(root);
                            }
                        }
                    }
                }
                _ => {
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op
                            && let Some(&root) = ptr_to_root.get(&v.0)
                        {
                            escaped.insert(root);
                        }
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op
                && let Some(&root) = ptr_to_root.get(&v.0)
            {
                escaped.insert(root);
            }
        });
    }
    escaped
}

fn analyze_out_param_summary(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
    out_param_fns: &FxHashMap<String, OutParamSummary>,
    lib: &LibraryFacts,
) -> Option<OutParamSummary> {
    if !func.is_static
        || func.is_weak
        || func.is_declaration
        || func.blocks.is_empty()
        || func.is_variadic
        || super::fp_environment_blocks_reordering(func)
        || func_has_back_edge(func)
    {
        return None;
    }

    let def_counts = collect_single_def_counts(func);
    let ptr_to_root = collect_local_alloca_roots(func, &def_counts);
    let escaped_roots = find_escaped_alloca_roots(func, &ptr_to_root, out_param_fns);
    let is_unescaped_local = |id: u32| -> bool {
        ptr_to_root
            .get(&id)
            .is_some_and(|root| !escaped_roots.contains(root))
    };

    let mut param_to_idx: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest,
                param_idx,
                ty: IrType::Ptr,
            } = inst
            {
                if *param_idx >= 64 || def_counts.get(&dest.0) != Some(&1) {
                    return None;
                }
                param_to_idx.insert(dest.0, *param_idx);
            }
        }
    }
    if param_to_idx.is_empty() {
        return None;
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    }
                    | Instruction::GetElementPtr {
                        dest, base: src, ..
                    } if def_counts.get(&dest.0) == Some(&1) => {
                        if let Some(&p) = param_to_idx.get(&src.0)
                            && param_to_idx.insert(dest.0, p) != Some(p)
                        {
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Ensure no pointer parameter (or derived pointer) ever escapes, except
    // when stored into a local unescaped slot or into a distinct out-parameter
    // (recorded in `captured_into_out_mask` so callers track local-alloca escape).
    let mut captured_into_out_mask = 0u64;
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::ParamRef { .. } => {}
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => {
                    if param_to_idx.contains_key(&src.0) && !param_to_idx.contains_key(&dest.0) {
                        return None;
                    }
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    if param_to_idx.contains_key(&base.0) && !param_to_idx.contains_key(&dest.0) {
                        return None;
                    }
                    if let Operand::Value(off) = offset
                        && param_to_idx.contains_key(&off.0)
                    {
                        return None;
                    }
                }
                Instruction::Load { .. } => {}
                Instruction::Store { ptr, val, .. } => {
                    if let Operand::Value(v) = val
                        && let Some(&src_p) = param_to_idx.get(&v.0)
                    {
                        if is_unescaped_local(ptr.0) {
                            // Stored into a non-escaping local slot of the helper itself.
                        } else if let Some(&dst_p) = param_to_idx.get(&ptr.0)
                            && dst_p != src_p
                        {
                            captured_into_out_mask |= 1u64 << src_p;
                        } else {
                            return None;
                        }
                    }
                }
                Instruction::Memcpy { .. } => {}
                Instruction::Call { func: callee, info }
                    if !info.is_sret
                        && !info.is_variadic
                        && out_param_fns.contains_key(callee.as_str()) =>
                {
                    let sub = out_param_fns[callee.as_str()];
                    if sub.captured_into_out_mask != 0 {
                        for (idx, arg) in info.args.iter().enumerate() {
                            if idx < 64
                                && ((sub.captured_into_out_mask >> idx) & 1) != 0
                                && let Operand::Value(v) = arg
                                && let Some(&src_p) = param_to_idx.get(&v.0)
                            {
                                // Conservative if any out-param of `sub` aliases `src_p`.
                                let aliases_self = (0..64usize).any(|b| {
                                    (sub.out_param_mask & (1u64 << b)) != 0
                                        && matches!(
                                            info.args.get(b),
                                            Some(Operand::Value(ov))
                                                if param_to_idx.get(&ov.0) == Some(&src_p)
                                        )
                                });
                                if aliases_self {
                                    return None;
                                }
                                let writes_param_out = (0..64usize).any(|b| {
                                    (sub.out_param_mask & (1u64 << b)) != 0
                                        && matches!(
                                            info.args.get(b),
                                            Some(Operand::Value(ov))
                                                if param_to_idx.contains_key(&ov.0)
                                        )
                                });
                                if writes_param_out {
                                    captured_into_out_mask |= 1u64 << src_p;
                                }
                            }
                        }
                    }
                }
                _ => {
                    let mut bad = false;
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op
                            && param_to_idx.contains_key(&v.0)
                        {
                            bad = true;
                        }
                    });
                    if bad {
                        return None;
                    }
                }
            }
        }
        let mut term_bad = false;
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op
                && param_to_idx.contains_key(&v.0)
            {
                term_bad = true;
            }
        });
        if term_bad {
            return None;
        }
    }

    let mut out_param_mask = 0u64;
    let mut reads_nonlocal = false;

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
                        return None;
                    }
                    if is_unescaped_local(ptr.0) {
                        // ok
                    } else if let Some(&p) = param_to_idx.get(&ptr.0) {
                        out_param_mask |= 1u64 << p;
                    } else {
                        return None;
                    }
                }
                Instruction::Load {
                    ptr,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *volatile || *seg_override != AddressSpace::Default {
                        return None;
                    }
                    if !is_unescaped_local(ptr.0) {
                        reads_nonlocal = true;
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    if is_unescaped_local(dest.0) {
                        // ok
                    } else if let Some(&p) = param_to_idx.get(&dest.0) {
                        out_param_mask |= 1u64 << p;
                    } else {
                        return None;
                    }
                    if !is_unescaped_local(src.0) {
                        reads_nonlocal = true;
                    }
                }
                Instruction::Call { func: callee, info } => {
                    let callee_const = const_fns.contains(callee.as_str())
                        || info.is_const
                        || lib.is_const(callee, info);
                    let callee_pure = pure_fns.contains(callee.as_str())
                        || info.is_pure
                        || lib.is_pure(callee, info);
                    if callee_const {
                        // ok
                    } else if callee_pure {
                        reads_nonlocal = true;
                    } else if !info.is_sret
                        && !info.is_variadic
                        && let Some(sub) = out_param_fns.get(callee.as_str())
                    {
                        for b in 0..64usize {
                            if sub.out_param_mask & (1u64 << b) != 0 {
                                let Some(Operand::Value(arg_v)) = info.args.get(b) else {
                                    return None;
                                };
                                if is_unescaped_local(arg_v.0) {
                                    // ok
                                } else if let Some(&p) = param_to_idx.get(&arg_v.0) {
                                    out_param_mask |= 1u64 << p;
                                } else {
                                    return None;
                                }
                            }
                        }
                        if sub.reads_nonlocal
                            || info.arg_types.iter().enumerate().any(|(i, ty)| {
                                *ty == IrType::Ptr
                                    && !matches!(
                                        info.args.get(i),
                                        Some(Operand::Value(v)) if is_unescaped_local(v.0)
                                    )
                            })
                        {
                            reads_nonlocal = true;
                        }
                    } else {
                        return None;
                    }
                }
                Instruction::CallIndirect { info, .. } => {
                    if info.is_const {
                        // ok
                    } else if info.is_pure {
                        reads_nonlocal = true;
                    } else {
                        return None;
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
                | Instruction::VaArgVector { .. }
                | Instruction::PgoCounterInc { .. }
                | Instruction::Fence { .. }
                | Instruction::InitTrampoline { .. }
                | Instruction::NonlocalGotoSave { .. }
                | Instruction::NonlocalGoto { .. }
                | Instruction::StackRestore { .. } => {
                    return None;
                }
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
                    return None;
                }
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
                | Instruction::SetStaticChain { .. } => {}
                Instruction::Intrinsic { op, dest_ptr, .. } => {
                    if !op.is_pure() || dest_ptr.is_some() {
                        return None;
                    }
                    if op.may_read_memory() {
                        reads_nonlocal = true;
                    }
                }
            }
        }
        if matches!(block.terminator, Terminator::IndirectBranch { .. }) {
            return None;
        }
    }

    (out_param_mask != 0).then_some(OutParamSummary {
        out_param_mask,
        captured_into_out_mask,
        reads_nonlocal,
    })
}

/// Analyze a function body to determine if it meets the criteria for `pure` and/or `const`.
#[cfg(test)]
fn analyze_function_purity(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
    lib: &LibraryFacts,
) -> (bool, bool) {
    analyze_function_purity_with_summaries(func, pure_fns, const_fns, &FxHashMap::default(), lib)
}

fn analyze_function_purity_with_summaries(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
    out_param_fns: &FxHashMap<String, OutParamSummary>,
    lib: &LibraryFacts,
) -> (bool, bool) {
    if super::fp_environment_blocks_reordering(func) {
        return (false, false);
    }

    let def_counts = collect_single_def_counts(func);
    let ptr_to_root = collect_local_alloca_roots(func, &def_counts);
    let escaped_roots = find_escaped_alloca_roots(func, &ptr_to_root, out_param_fns);
    let is_unescaped_local = |id: u32| -> bool {
        ptr_to_root
            .get(&id)
            .is_some_and(|root| !escaped_roots.contains(root))
    };

    let is_pure = true;
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
                    // Writing to a non-escaping local alloca (directly or via GEP/Copy) is allowed
                    // for pure/const functions. Writing to any other pointer invalidates both.
                    if !is_unescaped_local(ptr.0) {
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
                    if !is_unescaped_local(ptr.0) {
                        is_const = false;
                    }
                }
                Instruction::Call { func: callee, info } => {
                    let callee_const = const_fns.contains(callee.as_str())
                        || info.is_const
                        || lib.is_const(callee, info);
                    let callee_pure = pure_fns.contains(callee.as_str())
                        || info.is_pure
                        || lib.is_pure(callee, info);

                    if callee_const {
                        if info.arg_types.iter().enumerate().any(|(i, ty)| {
                            *ty == IrType::Ptr
                                && !matches!(
                                    info.args.get(i),
                                    Some(Operand::Value(v)) if is_unescaped_local(v.0)
                                )
                        }) {
                            is_const = false;
                        }
                    } else if callee_pure {
                        is_const = false;
                    } else if !info.is_sret
                        && !info.is_variadic
                        && let Some(summary) = out_param_fns.get(callee.as_str())
                    {
                        for b in 0..64usize {
                            if summary.out_param_mask & (1u64 << b) != 0 {
                                let Some(Operand::Value(arg_v)) = info.args.get(b) else {
                                    return (false, false);
                                };
                                if !is_unescaped_local(arg_v.0) {
                                    return (false, false);
                                }
                            }
                        }
                        if summary.reads_nonlocal
                            || info.arg_types.iter().enumerate().any(|(i, ty)| {
                                *ty == IrType::Ptr
                                    && !matches!(
                                        info.args.get(i),
                                        Some(Operand::Value(v)) if is_unescaped_local(v.0)
                                    )
                            })
                        {
                            is_const = false;
                        }
                    } else {
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
                    if !is_unescaped_local(dest.0) {
                        return (false, false);
                    }
                    if !is_unescaped_local(src.0) {
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
                | Instruction::VaArgVector { .. }
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
                | Instruction::SetStaticChain { .. } => {}
                // Intrinsics: purity excludes effects; constness also excludes reads.
                // A memory-writing intrinsic (Vec* stores, movnti, memcpy-
                // shaped helpers) or an observable one (fences, rdtsc,
                // clflush, vzeroupper) makes the function impure. The old
                // catch-all treated EVERY unlisted intrinsic as purity-
                // neutral, so a function whose memory writes all went
                // through vectorizer intrinsics (e.g. a loop-vectorized
                // `void fill(int*)`, or the BB-SLP 4×u64 copy) was marked
                // pure and its calls could be deleted. The runtime contract is
                // tests/regression/bb_slp_pure_store_fn.c; direct IR tests below
                // additionally exercise the args-only store form.
                Instruction::Intrinsic { op, dest_ptr, .. } => {
                    if !op.is_pure() || dest_ptr.is_some() {
                        return (false, false);
                    }
                    if op.may_read_memory() {
                        is_const = false;
                    }
                }
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

#[cfg(test)]
mod memory_read_tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::intrinsics::IntrinsicOp as O;
    use crate::ir::reexports::{BasicBlock, BlockId};

    #[test]
    fn store_effect_contract_function_is_neither_pure_nor_const() {
        for op in crate::ir::intrinsics::VECTOR_STORES {
            for pointer in [None, Some(crate::ir::reexports::Value(0))] {
                let mut f = IrFunction::new("writer".into(), IrType::Void, vec![], false);
                f.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions: vec![Instruction::Intrinsic {
                        dest: None,
                        op,
                        dest_ptr: pointer,
                        args: vec![
                            Operand::Value(crate::ir::reexports::Value(1)),
                            Operand::Value(crate::ir::reexports::Value(0)),
                            Operand::Const(crate::ir::reexports::IrConst::I64(0)),
                        ],
                    }],
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                });
                assert_eq!(
                    analyze_function_purity(
                        &f,
                        &FxHashSet::default(),
                        &FxHashSet::default(),
                        &LibraryFacts::default(),
                    ),
                    (false, false),
                    "{op:?}, {pointer:?}"
                );
            }
        }
    }

    #[test]
    fn legacy_source_reading_stores_are_not_pure_or_const() {
        use crate::ir::reexports::Value;

        for op in [O::StoreuPs256, O::StoreuPd256] {
            for source in [Value(0), Value(1)] {
                let params = (0..2)
                    .map(|_| crate::ir::reexports::IrParam {
                        ty: IrType::Ptr,
                        noalias: false,
                        struct_size: None,
                        struct_align: None,
                        param_align: None,
                        struct_eightbyte_classes: vec![],
                        is_f128_sse: false,
                        riscv_float_class: None,
                    })
                    .collect();
                let mut f = IrFunction::new("legacy_store".into(), IrType::Void, params, false);
                f.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions: vec![
                        Instruction::ParamRef {
                            dest: Value(0),
                            param_idx: 0,
                            ty: IrType::Ptr,
                        },
                        Instruction::ParamRef {
                            dest: Value(1),
                            param_idx: 1,
                            ty: IrType::Ptr,
                        },
                        Instruction::Intrinsic {
                            dest: None,
                            op,
                            dest_ptr: Some(Value(0)),
                            args: vec![Operand::Value(source)],
                        },
                    ],
                    terminator: Terminator::Return(None),
                    source_spans: vec![crate::common::source::Span::dummy(); 3],
                });
                f.next_value_id = 2;
                assert_eq!(
                    analyze_function_purity(
                        &f,
                        &FxHashSet::default(),
                        &FxHashSet::default(),
                        &LibraryFacts::default(),
                    ),
                    (false, false),
                    "{op:?}, source={source:?}"
                );
            }
        }
    }

    #[test]
    fn vector_readers_are_not_constant_functions() {
        for op in [
            O::VecLoadF64x2,
            O::VecLoadF64x4,
            O::VecLoadF32x4,
            O::VecLoadF32x8,
            O::VecLoadI32x4,
            O::VecLoadI32x8,
            O::VecLoadI64x2,
            O::VecLoadI64x4,
            O::VecLoadI16x8,
            O::VecLoadI8x16,
            O::VecLoadI8x32,
            O::VecLoadI16x16,
            O::VecLoadI32x4Pair,
            O::VecLoadWidenI32ToI64x2,
            O::SqrtF64,
        ] {
            let param_ty = if op.may_read_memory() {
                IrType::Ptr
            } else {
                IrType::F64
            };
            let mut f = IrFunction::new(
                "reader".into(),
                IrType::Void,
                vec![crate::ir::reexports::IrParam {
                    ty: param_ty,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    param_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    riscv_float_class: None,
                }],
                false,
            );
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::ParamRef {
                        dest: crate::ir::reexports::Value(0),
                        param_idx: 0,
                        ty: param_ty,
                    },
                    Instruction::Intrinsic {
                        dest: Some(crate::ir::reexports::Value(1)),
                        op,
                        dest_ptr: None,
                        args: vec![Operand::Value(crate::ir::reexports::Value(0))],
                    },
                ],
                terminator: Terminator::Return(None),
                source_spans: vec![],
            });
            // Analyze the pre-DCE IR; an unused nonvolatile reader may later
            // disappear, but it must not be classified as read-none while present.
            assert_eq!(
                analyze_function_purity(
                    &f,
                    &FxHashSet::default(),
                    &FxHashSet::default(),
                    &LibraryFacts::default(),
                ),
                (true, !op.may_read_memory()),
                "{op:?}"
            );
        }
    }

    #[test]
    fn out_param_captured_input_pointer_preserves_readonly_and_tracks_local_escape() {
        use crate::ir::reexports::Value;
        let ptr_param = || crate::ir::reexports::IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: vec![],
            is_f128_sse: false,
            riscv_float_class: None,
        };
        // Helper `parse_cell(buf, out_cell)` stores `buf` into `*out_cell`.
        let mut helper = IrFunction::new(
            "parse_cell".into(),
            IrType::Void,
            vec![ptr_param(), ptr_param()],
            false,
        );
        helper.is_static = true;
        helper.blocks.push(crate::ir::reexports::BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Value(Value(0)),
                    ptr: Value(1),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });

        let summary = analyze_out_param_summary(
            &helper,
            &FxHashSet::default(),
            &FxHashSet::default(),
            &FxHashMap::default(),
            &LibraryFacts::default(),
        )
        .expect("helper storing input ptr into out-param should summarize");
        assert_eq!(summary.out_param_mask, 1 << 1);
        assert_eq!(summary.captured_into_out_mask, 1 << 0);

        let mut out_map = FxHashMap::default();
        out_map.insert("parse_cell".to_string(), summary);

        // Read-only caller: passes external `buf` + local `&cell`, then reads `*cell.z`.
        let mut ro_caller = IrFunction::new("scan_ro".into(), IrType::U8, vec![ptr_param()], false);
        ro_caller.is_static = true;
        ro_caller.blocks.push(crate::ir::reexports::BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Call {
                    func: "parse_cell".into(),
                    info: CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(0)), Operand::Value(Value(1))],
                        arg_types: vec![IrType::Ptr, IrType::Ptr],
                        return_type: IrType::Void,
                        ..CallInfo::default()
                    },
                },
                Instruction::Load {
                    dest: Value(2),
                    ptr: Value(1),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(3),
                    ptr: Value(2),
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: vec![],
        });
        assert_eq!(
            analyze_function_purity_with_summaries(
                &ro_caller,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &out_map,
                &LibraryFacts::default(),
            ),
            (true, false)
        );

        // Mutating caller storing through loaded `cell.z` must NOT be pure.
        let mut mut_caller = ro_caller.clone();
        mut_caller.blocks[0].instructions.push(Instruction::Store {
            val: Operand::Const(crate::ir::reexports::IrConst::I8(1)),
            ptr: Value(2),
            ty: IrType::U8,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        assert_eq!(
            analyze_function_purity_with_summaries(
                &mut_caller,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &out_map,
                &LibraryFacts::default(),
            ),
            (false, false)
        );
    }
}
