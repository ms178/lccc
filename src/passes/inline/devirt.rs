//! Constant-target devirtualization for the inliner.
//!
//! An indirect call through a function pointer that is *provably* always
//! the same internal function is rewritten to a direct call, so the
//! ordinary inliner can see the callee.  Two layers cooperate:
//!
//! * `resolve_fn_ptr_origin` traces one argument back to a global symbol
//!   or a parameter through uniquely-defined copies and unescaped
//!   entry-block allocas (anything else is unknown, and unknown refuses);
//! * `propagate_invariant_fn_ptr_params` proves, across every call site
//!   in the module, that one parameter of an internal-linkage callee
//!   always carries the same symbol, iterating to a bounded fixpoint so
//!   the fact composes down passthrough call chains.
//!
//! Extracted from `inline.rs` (which keeps the cost model and the
//! inlining machinery proper) so the soundness fence of this transform
//! can be audited in one file; the refusal-space tests live with the
//! inliner's test modules and call through `super`.

use super::*;

/// Rewrite an indirect call whose target is provably one local function into
/// the equivalent direct call, so ordinary inlining can see it.
///
/// Template specialization often reaches the inliner as `call *%f` where `%f`
/// is a constant-folded `GlobalAddr`; the backend already emits that as a
/// direct `call f`, but the IR call graph still hides the callee. Rules:
/// * the pointer must resolve through uniquely-defined `Copy` values to one
///   `GlobalAddr`; reassigned or phi-joined values are not constants;
/// * the target must be a non-weak, non-variadic definition without an asm
///   label, so the direct name binds exactly where the address bound;
/// * the call-site ABI must be exactly the definition's scalar ABI. Struct,
///   sret, binary128-in-SSE and fastcall/regparm mismatches stay indirect.
///
#[derive(Clone, Debug, PartialEq, Eq)]
enum FnPtrOrigin {
    Global(String),
    Param(usize),
}

type ValueDefTable = FxHashMap<Value, (u32, Option<(usize, usize)>)>;

fn build_value_def_table(func: &IrFunction) -> ValueDefTable {
    let mut defs: ValueDefTable = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                let entry = defs.entry(dest).or_insert((0, None));
                entry.0 += 1;
                entry.1 = Some((bi, ii));
            }
        }
    }
    defs
}

fn terminator_targets_block(term: &Terminator, target: BlockId) -> bool {
    match term {
        Terminator::Branch(t) => *t == target,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == target || *false_label == target,
        Terminator::Switch { default, cases, .. } => {
            *default == target || cases.iter().any(|(_, l)| *l == target)
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.contains(&target),
        Terminator::Return(_) | Terminator::Unreachable => false,
    }
}

/// Entry-block allocas that are initialized by a single non-volatile `Store`
/// before any `Load` and never escape or get overwritten anywhere in `func`.
fn unescaped_entry_alloca_inits(func: &IrFunction) -> FxHashMap<Value, Value> {
    let Some(entry_block) = func.blocks.first() else {
        return FxHashMap::default();
    };
    let entry_label = entry_block.label;
    if func
        .blocks
        .iter()
        .any(|b| terminator_targets_block(&b.terminator, entry_label))
    {
        return FxHashMap::default();
    }
    let mut states: FxHashMap<Value, Option<Option<Value>>> = FxHashMap::default();
    for inst in &entry_block.instructions {
        if let Instruction::Alloca {
            dest,
            volatile: false,
            ..
        } = inst
        {
            states.entry(*dest).or_insert(Some(None));
        }
    }
    if states.is_empty() {
        return FxHashMap::default();
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        block.terminator.for_each_used_value(|v| {
            if let Some(slot) = states.get_mut(&Value(v)) {
                *slot = None;
            }
        });
        for inst in &block.instructions {
            match inst {
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(init_val),
                    ptr,
                    ..
                } if bi == 0 && states.contains_key(ptr) && init_val != ptr => {
                    if let Some(slot) = states.get_mut(init_val) {
                        *slot = None;
                    }
                    if let Some(slot) = states.get_mut(ptr) {
                        *slot = match *slot {
                            Some(None) => Some(Some(*init_val)),
                            _ => None,
                        };
                    }
                }
                Instruction::Load {
                    volatile: false,
                    ptr,
                    ..
                } if states.contains_key(ptr) => {
                    if bi == 0 && matches!(states.get(ptr), Some(Some(None))) {
                        states.insert(*ptr, None);
                    }
                }
                _ => {
                    inst.for_each_used_value(|v| {
                        if let Some(slot) = states.get_mut(&Value(v)) {
                            *slot = None;
                        }
                    });
                }
            }
        }
    }
    states
        .into_iter()
        .filter_map(|(k, v)| Some((k, v??)))
        .collect()
}

/// Hard bound on the provenance links `resolve_fn_ptr_origin` will follow
/// (copy/load hops between a call argument and its origin).  Real chains
/// are one to three links; beyond the bound the answer is "unknown", which
/// the caller treats as a refusal -- a missed devirtualization, never a
/// wrong one.
const MAX_FN_PTR_ORIGIN_STEPS: usize = 16;

/// Hard bound on the agreement-propagation rounds in
/// `propagate_invariant_fn_ptr_params`.  Each round resolves one more
/// level of call-chain indirection, so the bound caps chain depth, not
/// program size; exhausting it leaves the deepest parameters unknown
/// (again: refusal, not unsoundness).
const MAX_INVARIANT_FIXPOINT_ROUNDS: usize = 8;

fn resolve_fn_ptr_origin(
    func: &IrFunction,
    defs: &ValueDefTable,
    alloca_inits: &FxHashMap<Value, Value>,
    mut value: Value,
) -> Option<FnPtrOrigin> {
    for _ in 0..MAX_FN_PTR_ORIGIN_STEPS {
        let &(count, Some((bi, ii))) = defs.get(&value)? else {
            return None;
        };
        if count != 1 {
            return None;
        }
        match &func.blocks[bi].instructions[ii] {
            Instruction::GlobalAddr { name, .. } => return Some(FnPtrOrigin::Global(name.clone())),
            Instruction::ParamRef { param_idx, .. } => return Some(FnPtrOrigin::Param(*param_idx)),
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => value = *src,
            Instruction::Load {
                ptr,
                volatile: false,
                ..
            } => value = *alloca_inits.get(ptr)?,
            _ => return None,
        }
    }
    None
}

/// Propagate function-pointer arguments across internal-linkage functions whose
/// address is never taken and whose callers all pass the same target symbol.
fn propagate_invariant_fn_ptr_params(
    module: &IrModule,
    eligible_targets: &FxHashSet<String>,
    func_tables: &[(ValueDefTable, FxHashMap<Value, Value>)],
) -> FxHashMap<(String, usize), String> {
    let (value_referenced, asm_templates) = collect_value_referenced_functions(module);
    let mut eligible_callees: FxHashMap<String, Vec<bool>> = module
        .functions
        .iter()
        .filter(|f| {
            f.is_static
                && !f.is_declaration
                && !f.is_weak
                && !f.is_variadic
                && !module.asm_labels.contains_key(&f.name)
                && !function_referenced_as_value(&f.name, &value_referenced, &asm_templates)
        })
        .map(|f| {
            let scalar_params = f
                .params
                .iter()
                .map(|p| p.struct_size.is_none() && !p.is_f128_sse)
                .collect();
            (f.name.clone(), scalar_params)
        })
        .collect();
    if eligible_callees.is_empty() {
        return FxHashMap::default();
    }
    let mut call_sites: FxHashMap<(String, usize), Vec<(String, Option<FnPtrOrigin>)>> =
        FxHashMap::default();
    for (fi, caller) in module.functions.iter().enumerate() {
        if caller.is_declaration {
            continue;
        }
        let (defs, alloca_inits) = &func_tables[fi];
        for block in &caller.blocks {
            for inst in &block.instructions {
                let Instruction::Call {
                    func: callee_name,
                    info,
                } = inst
                else {
                    continue;
                };
                let Some(param_mask) = eligible_callees.get(callee_name) else {
                    continue;
                };
                if info.is_variadic || info.args.len() != param_mask.len() {
                    eligible_callees.remove(callee_name);
                    continue;
                }
                for (pi, &is_scalar) in param_mask.iter().enumerate() {
                    if !is_scalar {
                        continue;
                    }
                    let origin = match &info.args[pi] {
                        Operand::Value(v) => resolve_fn_ptr_origin(caller, defs, alloca_inits, *v),
                        Operand::Const(_) => None,
                    };
                    call_sites
                        .entry((callee_name.clone(), pi))
                        .or_default()
                        .push((caller.name.clone(), origin));
                }
            }
        }
    }
    call_sites
        .retain(|(callee, _), sites| eligible_callees.contains_key(callee) && !sites.is_empty());
    let mut known: FxHashMap<(String, usize), String> = FxHashMap::default();
    for _ in 0..MAX_INVARIANT_FIXPOINT_ROUNDS {
        let mut progressed = false;
        for ((callee, pi), sites) in &call_sites {
            if known.contains_key(&(callee.clone(), *pi)) {
                continue;
            }
            let mut agreed: Option<&str> = None;
            let mut all_ok = true;
            for (caller_name, origin_opt) in sites {
                let Some(origin) = origin_opt else {
                    all_ok = false;
                    break;
                };
                if caller_name == callee && *origin == FnPtrOrigin::Param(*pi) {
                    continue;
                }
                let target = match origin {
                    FnPtrOrigin::Global(sym) => sym.as_str(),
                    FnPtrOrigin::Param(caller_pi) => {
                        let Some(sym) = known.get(&(caller_name.clone(), *caller_pi)) else {
                            all_ok = false;
                            break;
                        };
                        sym.as_str()
                    }
                };
                match agreed {
                    None => agreed = Some(target),
                    Some(prev) if prev == target => {}
                    Some(_) => {
                        all_ok = false;
                        break;
                    }
                }
            }
            if all_ok
                && let Some(target) = agreed
                && eligible_targets.contains(target)
            {
                known.insert((callee.clone(), *pi), target.to_string());
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    known
}

/// A call through a mismatched prototype is undefined in C, but rewriting it
/// would turn that into inliner parameter-type confusion. Refusing is cheap.
pub(super) fn devirtualize_constant_indirect_calls(
    module: &mut IrModule,
) -> FxHashSet<(String, String)> {
    struct Target {
        params: Vec<IrType>,
        return_type: IrType,
        calling_convention: crate::common::types::CallingConvention,
        is_fastcall: bool,
        regparm: Option<u8>,
    }
    let targets: FxHashMap<String, Target> = module
        .functions
        .iter()
        .filter(|f| {
            !f.is_declaration
                && !f.is_weak
                && !f.is_variadic
                && !f.uses_sret
                && !f.ret_is_f128_sse
                && f.ret_eightbyte_classes.is_empty()
                && f.params
                    .iter()
                    .all(|p| p.struct_size.is_none() && !p.is_f128_sse)
                && !module.asm_labels.contains_key(&f.name)
        })
        .map(|f| {
            (
                f.name.clone(),
                Target {
                    params: f.params.iter().map(|p| p.ty).collect(),
                    return_type: f.return_type,
                    calling_convention: f.calling_convention,
                    is_fastcall: f.is_fastcall,
                    regparm: f.regparm,
                },
            )
        })
        .collect();
    let mut rewritten: FxHashSet<(String, String)> = FxHashSet::default();
    if targets.is_empty() {
        return rewritten;
    }
    let eligible_target_names: FxHashSet<String> = targets.keys().cloned().collect();
    let func_tables: Vec<(ValueDefTable, FxHashMap<Value, Value>)> = module
        .functions
        .iter()
        .map(|f| {
            if f.is_declaration {
                (FxHashMap::default(), FxHashMap::default())
            } else {
                (build_value_def_table(f), unescaped_entry_alloca_inits(f))
            }
        })
        .collect();
    let known_params =
        propagate_invariant_fn_ptr_params(module, &eligible_target_names, &func_tables);
    for (fi, func) in module.functions.iter_mut().enumerate() {
        if func.is_declaration {
            continue;
        }
        let (defs, alloca_inits) = &func_tables[fi];
        let mut rewrites: Vec<(usize, usize, String)> = Vec::new();
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                let Instruction::CallIndirect {
                    func_ptr: Operand::Value(ptr),
                    info,
                } = inst
                else {
                    continue;
                };
                let name = match resolve_fn_ptr_origin(func, defs, alloca_inits, *ptr) {
                    Some(FnPtrOrigin::Global(sym)) => Some(sym),
                    Some(FnPtrOrigin::Param(pi)) => {
                        known_params.get(&(func.name.clone(), pi)).cloned()
                    }
                    None => None,
                };
                let Some(name) = name else {
                    continue;
                };
                let Some(target) = targets.get(&name) else {
                    continue;
                };
                let scalar_abi = !info.is_variadic
                    && !info.is_sret
                    && !info.ret_is_f128_sse
                    && info.ret_eightbyte_classes.is_empty()
                    && info.struct_arg_sizes.iter().all(Option::is_none)
                    && info.struct_arg_is_f128_sse.iter().all(|f| !f)
                    && info.args.len() == target.params.len()
                    && info.arg_types == target.params
                    && info.num_fixed_args == target.params.len()
                    && info.return_type == target.return_type
                    && info.calling_convention == target.calling_convention
                    && info.is_fastcall == target.is_fastcall
                    && info.regparm == target.regparm;
                if scalar_abi {
                    rewrites.push((bi, ii, name));
                }
            }
        }
        for (bi, ii, name) in rewrites {
            let inst = &mut func.blocks[bi].instructions[ii];
            if let Instruction::CallIndirect { info, .. } = inst {
                let info = info.clone();
                rewritten.insert((func.name.clone(), name.clone()));
                *inst = Instruction::Call { func: name, info };
            }
        }
    }
    rewritten
}
