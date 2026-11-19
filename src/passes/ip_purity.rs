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
//!
//! Two facts are kept apart, as GCC keeps them (`looping` pure/const):
//!
//! - what the function does to memory on the executions that return
//!   ([`MemoryEffect`], stamped as `CallInfo::memory`), and
//! - whether every execution returns ([`body_always_returns`], and every
//!   callee does).
//!
//! Only both together make a call `is_pure`/`is_const`: those flags license
//! deleting a call whose result is unused and moving one out of a loop, and a
//! trap or a loop without end is observable although it writes nothing.  The
//! memory fact alone is still worth having -- a second `check(i, n)` or a
//! second `length(s)` can reuse the first, and what was loaded before the
//! call is still current after it -- so it is recorded for the calls that
//! cannot be marked pure.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::{self, FlatAdj};
use crate::ir::reexports::{
    CallInfo, Instruction, IrFunction, IrModule, MemoryEffect, Operand, Terminator,
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
                    total_updates += usize::from(info.memory_effect() != MemoryEffect::ReadWrite);
                    info.is_pure = false;
                    info.is_const = false;
                    info.memory = MemoryEffect::ReadWrite;
                }
            }
        }
    }
    let mut pure_fns: FxHashSet<String> = FxHashSet::default();
    let mut const_fns: FxHashSet<String> = FxHashSet::default();
    // Functions that write nothing but are not known to return: their memory
    // effect, never `ReadWrite`.  A function shown to return is in the sets
    // above, and those are consulted first.
    let mut quiet_fns: FxHashMap<String, MemoryEffect> = FxHashMap::default();
    // What the C library guarantees about its own functions.  Kept apart
    // from the inferred/annotated sets: it describes the LIBRARY function,
    // so it applies only to calls known to reach it
    // (`CallInfo::assume_library` -- false under `-fno-builtin[-<fn>]` /
    // `-ffreestanding`), never to a name.
    let mut lib = LibraryFacts::default();

    // Standard math builtins known to be const
    for name in TOTAL_LIBRARY_FUNCTIONS {
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

    // Collect initial annotations from existing CallInfo in the module: a
    // declared attribute, or what an earlier run of this pass inferred.
    //
    // A call the library facts account for says nothing about the *name*.  It
    // is `__builtin_strlen(s)`, or a plain call the builtin policy let through;
    // under `-fno-builtin-strlen` or `-ffreestanding` another call of that
    // name is to whatever the program links, and importing the fact here
    // marked every one of them pure after the first run -- two calls to a
    // `strlen` that counts its invocations were merged into one as soon as
    // the translation unit also spelled `__builtin_strlen` somewhere.
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Call { func: fname, info } = inst {
                    if lib.is_pure(fname, info) {
                        continue;
                    }
                    if info.is_const {
                        const_fns.insert(fname.clone());
                        pure_fns.insert(fname.clone());
                    } else if info.is_pure {
                        pure_fns.insert(fname.clone());
                    } else if info.memory != MemoryEffect::ReadWrite {
                        record_quiet(&mut quiet_fns, fname, info.memory);
                    }
                }
            }
        }
    }

    // Iteratively infer const, pure, and non-capturing out-param summaries for defined functions
    let mut out_param_fns: FxHashMap<String, OutParamSummary> = FxHashMap::default();
    // Whether each body returns ([`body_always_returns`]), once it was asked.
    let mut body_returns: Vec<Option<bool>> = vec![None; module.functions.len()];
    loop {
        let mut progress = false;

        for (func, body_returns) in module.functions.iter().zip(&mut body_returns) {
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

            let facts = analyze_function_body(
                func,
                &pure_fns,
                &const_fns,
                &quiet_fns,
                &out_param_fns,
                &lib,
                body_returns,
            );

            if facts.is_const() {
                const_fns.insert(func.name.clone());
                pure_fns.insert(func.name.clone());
                progress = true;
            } else if facts.is_pure() {
                if !is_already_pure {
                    pure_fns.insert(func.name.clone());
                    progress = true;
                }
            } else if is_already_pure {
                // Declared pure, or found pure before a later pass reshaped
                // the body: nothing weaker is worth recording.
            } else if facts.memory != MemoryEffect::ReadWrite {
                progress |= record_quiet(&mut quiet_fns, &func.name, facts.memory);
            } else if func.is_static
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
                    } else if let Some(&memory) = quiet_fns.get(callee.as_str())
                        && stronger(memory, info.memory)
                    {
                        info.memory = memory;
                        total_updates += 1;
                    }
                }
            }
        }
    }

    total_updates
}

/// Whether memory effect `a` promises strictly more than `b`.
fn stronger(a: MemoryEffect, b: MemoryEffect) -> bool {
    let rank = |effect| match effect {
        MemoryEffect::ReadWrite => 0,
        MemoryEffect::ReadOnly => 1,
        MemoryEffect::ReadNone => 2,
    };
    rank(a) > rank(b)
}

/// Record `memory` for `name` if it says more than what is known; whether it
/// did.
fn record_quiet(
    quiet_fns: &mut FxHashMap<String, MemoryEffect>,
    name: &str,
    memory: MemoryEffect,
) -> bool {
    let known = quiet_fns.get(name).copied().unwrap_or_default();
    if !stronger(memory, known) {
        return false;
    }
    quiet_fns.insert(name.to_string(), memory);
    true
}

/// What a function's body lets its callers assume about a call to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BodyFacts {
    /// Its effect on memory the caller can see, on the executions that return.
    memory: MemoryEffect,
    /// Every execution returns: the body does ([`body_always_returns`]), and so
    /// does everything it calls.
    returns: bool,
}

impl BodyFacts {
    /// Nothing can be assumed.
    const UNKNOWN: Self = Self {
        memory: MemoryEffect::ReadWrite,
        returns: false,
    };

    fn is_pure(self) -> bool {
        self.returns && self.memory != MemoryEffect::ReadWrite
    }

    fn is_const(self) -> bool {
        self.returns && self.memory == MemoryEffect::ReadNone
    }
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

/// The C library functions this pass treats as `const` when a call is known
/// to reach the library (`CallInfo::assume_library`).
///
/// Each is defined for every argument -- a domain error is a NaN, not a
/// fault -- which is more than `const` promises and is what
/// [`library_function_is_total`] answers for loop-invariant code motion.  A
/// function that divides its arguments (`div`, `ldiv`) does not belong here.
const TOTAL_LIBRARY_FUNCTIONS: &[&str] = &[
    "abs", "labs", "llabs", "imaxabs", "fabs", "fabsf", "fabsl", "sqrt", "sqrtf", "sqrtl", "sin",
    "sinf", "sinl", "cos", "cosf", "cosl", "tan", "tanf", "tanl", "ceil", "ceilf", "ceill",
    "floor", "floorf", "floorl", "trunc", "truncf", "truncl", "round", "roundf", "roundl", "log",
    "logf", "logl", "exp", "expf", "expl", "pow", "powf", "powl",
];

/// Whether the C library function `name` is defined for every argument: a
/// call to it cannot fault, so one that is also `const` may run where the
/// program would not have run it.  Says nothing about a function that is
/// merely *called* `name`; the caller checks `CallInfo::assume_library`.
pub(super) fn library_function_is_total(name: &str) -> bool {
    TOTAL_LIBRARY_FUNCTIONS.contains(&name)
}

/// What the flow graph of a body says about control coming back.
struct Flow {
    preds: FlatAdj,
    succs: FlatAdj,
    /// The blocks the entry reaches.  A dead block describes no execution,
    /// whatever it ends in: lowering leaves them behind until the CFG is
    /// cleaned.
    reachable: Vec<bool>,
    /// Some path from the entry runs through a block twice.
    cyclic: bool,
}

/// The flow of `func`, or `None` if a reachable block ends in a trap.
///
/// The edges are the function's own ([`analysis::build_cfg`]), the ones an
/// asm goto takes ahead of its block's terminator included: a block that only
/// such an edge reaches is as live as any other, and so is a cycle it closes.
///
/// Iterative three-colour DFS: recursion depth here would be proportional to
/// the block count, and a large machine generated function (or a
/// switch-dispatch state machine) would overflow the compiler's own stack on
/// a valid input.
fn flow_of(func: &IrFunction) -> Option<Flow> {
    let (preds, succs) = analysis::build_cfg(func, &analysis::build_label_map(func));
    let traps = |u: usize| matches!(func.blocks[u].terminator, Terminator::Unreachable);
    let blocks = func.blocks.len();
    let mut cyclic = false;
    // 0 = white (unvisited), 1 = gray (on the DFS path), 2 = black (finished).
    let mut color = vec![0u8; blocks];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    if blocks != 0 {
        if traps(0) {
            return None;
        }
        color[0] = 1;
        stack.push((0, 0));
    }
    // Each iteration copies the frame out of the stack before touching it,
    // so the `push` below never overlaps a live `&mut` into the vector.
    while let Some(&(u, next)) = stack.last() {
        let Some(&v) = succs.row(u).get(next) else {
            color[u] = 2;
            stack.pop();
            continue;
        };
        let v = v as usize;
        if let Some(top) = stack.last_mut() {
            top.1 = next + 1;
        }
        match color[v] {
            1 => cyclic = true,
            0 => {
                if traps(v) {
                    return None;
                }
                color[v] = 1;
                stack.push((v, 0));
            }
            _ => {}
        }
    }
    Some(Flow {
        preds,
        succs,
        reachable: color.into_iter().map(|c| c != 0).collect(),
        cyclic,
    })
}

/// Whether control comes back from every execution of `func`, as far as its
/// own body decides: no reachable block ends in a trap, and every loop ends.
///
/// `pure` and `const` are what license *deleting* a call whose result is
/// unused and *moving* one out of a loop, so they promise more than the
/// absence of side effects: the call has to hand control back.  A trap and a
/// loop that does not end are both observable, and neither writes memory.
/// Inferring the attributes from memory effects alone deleted the check in
///
/// ```c
/// static void check(unsigned i, unsigned n) { if (i >= n) __builtin_trap(); }
/// int get(const int *a, unsigned i, unsigned n) { check(i, n); return a[i]; }
/// ```
///
/// at `-O1` and above (GCC 16.2 and Clang 23.1.5 trap), and the call to a
/// function that parks in `for (;;) {}`.  GCC keeps such a function "looping
/// const" and LLVM withholds `willreturn`; both then keep the call.
///
/// A loop ends if it can be counted ([`super::loop_termination`]).  C11
/// 6.8.5p6 would let an implementation assume as much of any loop whose
/// controlling expression is not a constant, but the flow graph no longer
/// says which spelling a cycle came from: `for (;;) { if (c) break; }` and
/// `while (!c) {}` are the same blocks.  GCC makes the same choice for C: a
/// loop it cannot count makes the function "looping".
///
/// Callees are the caller's concern ([`analyze_function_body`]).  The
/// interprocedural constant propagation asks the same question of the
/// functions whose calls it folds away.
pub(super) fn body_always_returns(func: &IrFunction) -> bool {
    flow_of(func).is_some_and(|flow| {
        !flow.cyclic
            || super::loop_termination::every_cycle_ends(
                func,
                &flow.preds,
                &flow.succs,
                &flow.reachable,
            )
    })
}

/// Whether `func` returns without ever running a block twice: no reachable
/// trap and no cycle at all.
fn body_is_straight_line(func: &IrFunction) -> bool {
    flow_of(func).is_some_and(|flow| !flow.cyclic)
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
        // Summaries have only ever been made of bodies without a cycle, and
        // that a loop can be counted is no evidence for widening them.  The
        // callers below also read a summary as "the call returns", so a
        // reachable trap rules the body out as well.
        || !body_is_straight_line(func)
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

/// `(pure, const)` for a body whose callees are pure, const or unknown.
#[cfg(test)]
fn analyze_function_purity(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
    lib: &LibraryFacts,
) -> (bool, bool) {
    let facts = analyze_function_body(
        func,
        pure_fns,
        const_fns,
        &FxHashMap::default(),
        &FxHashMap::default(),
        lib,
        &mut None,
    );
    (facts.is_pure(), facts.is_const())
}

/// What `func`'s body does to memory and whether it always returns, given
/// what is known of its callees:
///
/// - `pure_fns`/`const_fns`, a call's own `is_pure`/`is_const`, and the
///   library facts name callees that return;
/// - `quiet_fns` and a call's `memory` name callees that write nothing but may
///   not return, and a function that calls one may not return either;
/// - `out_param_fns` summarises callees that write only through pointer
///   parameters (and return: [`analyze_out_param_summary`] admits no other).
///
/// Any other call, and anything else that writes non-local memory or has an
/// effect of its own, makes the answer [`BodyFacts::UNKNOWN`].
///
/// `body_returns` remembers [`body_always_returns`] for `func` across the
/// rounds of the fixpoint; pass `&mut None` to ask afresh.
fn analyze_function_body(
    func: &IrFunction,
    pure_fns: &FxHashSet<String>,
    const_fns: &FxHashSet<String>,
    quiet_fns: &FxHashMap<String, MemoryEffect>,
    out_param_fns: &FxHashMap<String, OutParamSummary>,
    lib: &LibraryFacts,
    body_returns: &mut Option<bool>,
) -> BodyFacts {
    if super::fp_environment_blocks_reordering(func) {
        return BodyFacts::UNKNOWN;
    }
    // Whether every callee returns; the body's own answer is asked last.
    let mut callees_return = true;

    let def_counts = collect_single_def_counts(func);
    let ptr_to_root = collect_local_alloca_roots(func, &def_counts);
    let escaped_roots = find_escaped_alloca_roots(func, &ptr_to_root, out_param_fns);
    let is_unescaped_local = |id: u32| -> bool {
        ptr_to_root
            .get(&id)
            .is_some_and(|root| !escaped_roots.contains(root))
    };

    // Whether anything read may be memory the caller can see.
    let mut reads = false;

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
                        return BodyFacts::UNKNOWN;
                    }
                    // Writing to a non-escaping local alloca (directly or via GEP/Copy) is allowed
                    // for pure/const functions. Writing to any other pointer invalidates both.
                    if !is_unescaped_local(ptr.0) {
                        return BodyFacts::UNKNOWN;
                    }
                }
                Instruction::Load {
                    ptr,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *volatile || *seg_override != AddressSpace::Default {
                        return BodyFacts::UNKNOWN;
                    }
                    // Reading from non-local memory (globals, params, heap) is allowed for `pure`
                    // but not for `const`.
                    if !is_unescaped_local(ptr.0) {
                        reads = true;
                    }
                }
                Instruction::Call { func: callee, info } => {
                    let callee_const = const_fns.contains(callee.as_str())
                        || info.is_const
                        || lib.is_const(callee, info);
                    let callee_pure = pure_fns.contains(callee.as_str())
                        || info.is_pure
                        || lib.is_pure(callee, info);
                    // A callee that writes nothing but is not known to
                    // return: the same effect on memory, and this function
                    // may not return either.
                    let quiet = if callee_const || callee_pure {
                        MemoryEffect::ReadWrite
                    } else {
                        let named = quiet_fns.get(callee.as_str()).copied().unwrap_or_default();
                        if stronger(info.memory, named) {
                            info.memory
                        } else {
                            named
                        }
                    };
                    if quiet != MemoryEffect::ReadWrite {
                        callees_return = false;
                    }

                    if callee_const || quiet == MemoryEffect::ReadNone {
                        if info.arg_types.iter().enumerate().any(|(i, ty)| {
                            *ty == IrType::Ptr
                                && !matches!(
                                    info.args.get(i),
                                    Some(Operand::Value(v)) if is_unescaped_local(v.0)
                                )
                        }) {
                            reads = true;
                        }
                    } else if callee_pure || quiet == MemoryEffect::ReadOnly {
                        reads = true;
                    } else if !info.is_sret
                        && !info.is_variadic
                        && let Some(summary) = out_param_fns.get(callee.as_str())
                    {
                        for b in 0..64usize {
                            if summary.out_param_mask & (1u64 << b) != 0 {
                                let Some(Operand::Value(arg_v)) = info.args.get(b) else {
                                    return BodyFacts::UNKNOWN;
                                };
                                if !is_unescaped_local(arg_v.0) {
                                    return BodyFacts::UNKNOWN;
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
                            reads = true;
                        }
                    } else {
                        return BodyFacts::UNKNOWN;
                    }
                }
                Instruction::CallIndirect { info, .. } => {
                    match info.memory_effect() {
                        MemoryEffect::ReadWrite => return BodyFacts::UNKNOWN,
                        MemoryEffect::ReadOnly => reads = true,
                        MemoryEffect::ReadNone => {}
                    }
                    if !info.is_pure && !info.is_const {
                        callees_return = false;
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    if !is_unescaped_local(dest.0) {
                        return BodyFacts::UNKNOWN;
                    }
                    if !is_unescaped_local(src.0) {
                        reads = true;
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
                    return BodyFacts::UNKNOWN;
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
                    return BodyFacts::UNKNOWN;
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
                        return BodyFacts::UNKNOWN;
                    }
                    if op.may_read_memory() {
                        reads = true;
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
                return BodyFacts::UNKNOWN;
            }
        }
    }

    BodyFacts {
        memory: if reads {
            MemoryEffect::ReadOnly
        } else {
            MemoryEffect::ReadNone
        },
        // Asked only of a body that writes nothing, and once: it is a fact
        // about the body alone, and counting its loops takes dominators.
        returns: callees_return && *body_returns.get_or_insert_with(|| body_always_returns(func)),
    }
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
                    source_inline_ids: Vec::new(),
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
                        fp_agg_class: None,
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
                    source_inline_ids: Vec::new(),
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
                    fp_agg_class: None,
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
                source_inline_ids: Vec::new(),
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
            fp_agg_class: None,
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
        });
        let purity = |func: &IrFunction| {
            let facts = analyze_function_body(
                func,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &FxHashMap::default(),
                &out_map,
                &LibraryFacts::default(),
                &mut None,
            );
            (facts.is_pure(), facts.is_const())
        };
        assert_eq!(purity(&ro_caller), (true, false));

        // Mutating caller storing through loaded `cell.z` must NOT be pure.
        let mut mut_caller = ro_caller.clone();
        mut_caller.blocks[0].instructions.push(Instruction::Store {
            val: Operand::Const(crate::ir::reexports::IrConst::I8(1)),
            ptr: Value(2),
            ty: IrType::U8,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        assert_eq!(purity(&mut_caller), (false, false));
    }
}

#[cfg(test)]
mod always_returns_tests {
    //! `pure`/`const` promise that the call comes back: a trap and a loop that
    //! may not end are observable without writing memory.
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Value};

    fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions,
            terminator,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    fn ret(value: i32) -> Terminator {
        Terminator::Return(Some(Operand::Const(IrConst::I32(value))))
    }

    fn branch(target: u32) -> Terminator {
        Terminator::Branch(BlockId(target))
    }

    fn either(on_true: u32, on_false: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(0)),
            true_label: BlockId(on_true),
            false_label: BlockId(on_false),
        }
    }

    fn function(name: &str, blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
        f.is_static = true;
        f.blocks = blocks;
        f
    }

    fn purity(blocks: Vec<BasicBlock>) -> (bool, bool) {
        analyze_function_purity(
            &function("f", blocks),
            &FxHashSet::default(),
            &FxHashSet::default(),
            &LibraryFacts::default(),
        )
    }

    #[test]
    fn a_body_that_only_branches_forward_is_const() {
        assert_eq!(purity(vec![block(0, vec![], ret(1))]), (true, true));
        // A diamond reaches its join twice; that is not a cycle.
        let diamond = vec![
            block(0, vec![], either(1, 2)),
            block(1, vec![], branch(3)),
            block(2, vec![], branch(3)),
            block(3, vec![], ret(7)),
        ];
        assert_eq!(purity(diamond), (true, true));
    }

    /// `if (i >= n) __builtin_trap();` -- the whole point of the call.
    #[test]
    fn a_reachable_trap_is_neither_pure_nor_const() {
        let check = vec![
            block(0, vec![], either(1, 2)),
            block(1, vec![], Terminator::Unreachable),
            block(2, vec![], ret(0)),
        ];
        assert_eq!(purity(check), (false, false));
        assert_eq!(
            purity(vec![block(0, vec![], Terminator::Unreachable)]),
            (false, false)
        );
    }

    #[test]
    fn a_cycle_that_cannot_be_counted_is_neither_pure_nor_const() {
        // `for (;;) {}`: no exit at all.
        let parks = vec![block(0, vec![], branch(1)), block(1, vec![], branch(1))];
        assert_eq!(purity(parks), (false, false));
        // A loop with an exit, on a condition that counts nothing.
        let waits = vec![
            block(0, vec![], branch(1)),
            block(1, vec![], either(1, 2)),
            block(2, vec![], ret(0)),
        ];
        assert_eq!(purity(waits), (false, false));
        // The back edge closes over more than one block.
        let longer = vec![
            block(0, vec![], branch(1)),
            block(1, vec![], branch(2)),
            block(2, vec![], either(1, 3)),
            block(3, vec![], ret(0)),
        ];
        assert_eq!(purity(longer), (false, false));
    }

    /// `for (i = 0; i OP n; i += step) {}` with the bound a parameter:
    /// 0 entry, 1 header and test, 2 latch, 3 return.
    fn counts(op: crate::ir::reexports::IrCmpOp, step: i32) -> Vec<BasicBlock> {
        use crate::ir::reexports::IrBinOp;
        let int = |n| Operand::Const(IrConst::I32(n));
        let v = |n| Operand::Value(Value(n));
        vec![
            block(0, vec![param(9)], branch(1)),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(1),
                        ty: IrType::I32,
                        incoming: vec![(int(0), BlockId(0)), (v(2), BlockId(2))],
                    },
                    Instruction::Cmp {
                        dest: Value(0),
                        op,
                        lhs: v(1),
                        rhs: v(9),
                        ty: IrType::I32,
                    },
                ],
                either(2, 3),
            ),
            block(
                2,
                vec![Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: v(1),
                    rhs: int(step),
                    ty: IrType::I32,
                }],
                branch(1),
            ),
            block(3, vec![], ret(0)),
        ]
    }

    /// A loop that counts to its bound ends, and the function that holds it
    /// is as removable as one without a loop
    /// (tests/regression/costaware_devirt.c lost a 3000-fold hoist to "every
    /// cycle may not end").  The proof itself is [`super::super::loop_termination`]'s.
    #[test]
    fn a_loop_that_can_be_counted_keeps_the_function_const() {
        use crate::ir::reexports::IrCmpOp;
        assert_eq!(purity(counts(IrCmpOp::Slt, 1)), (true, true));
        assert_eq!(purity(counts(IrCmpOp::Ne, 1)), (true, true));
        // `i <= n` runs forever for the largest `n`, and a step of two can
        // pass the last value below the bound and wrap.
        assert_eq!(purity(counts(IrCmpOp::Sle, 1)), (false, false));
        assert_eq!(purity(counts(IrCmpOp::Slt, 2)), (false, false));
        // A trap behind a loop that ends is still a trap.
        let mut traps = counts(IrCmpOp::Slt, 1);
        traps[3].terminator = Terminator::Unreachable;
        assert_eq!(purity(traps), (false, false));
    }

    fn asm_goto(target: u32) -> Instruction {
        Instruction::InlineAsm {
            template: "jmp %l0".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("target".into(), BlockId(target))],
            input_symbols: vec![],
            seg_overrides: vec![],
        }
    }

    /// An asm goto leaves its block by an edge the terminator does not name.
    /// The interprocedural constant propagation asks this of functions it
    /// has no other reason to distrust, so the edge has to count here: the
    /// trap it reaches is live, and so is the cycle it closes.
    #[test]
    fn an_asm_goto_edge_is_an_edge() {
        let trap_behind_the_label = function(
            "f",
            vec![
                block(0, vec![asm_goto(2)], branch(1)),
                block(1, vec![], ret(0)),
                block(2, vec![], Terminator::Unreachable),
            ],
        );
        assert!(!body_always_returns(&trap_behind_the_label));
        let cycle_through_the_label = function(
            "f",
            vec![
                block(0, vec![], branch(1)),
                block(1, vec![asm_goto(1)], branch(2)),
                block(2, vec![], ret(0)),
            ],
        );
        assert!(!body_always_returns(&cycle_through_the_label));
        // The same blocks without the asm return.
        let plain = function(
            "f",
            vec![
                block(0, vec![], branch(1)),
                block(1, vec![], ret(0)),
                block(2, vec![], Terminator::Unreachable),
            ],
        );
        assert!(body_always_returns(&plain));
    }

    /// Whether the body returns is asked last, of a body that writes
    /// nothing, and the answer is kept for the next round of the fixpoint.
    #[test]
    fn the_body_is_asked_once_and_only_if_it_matters() {
        use crate::ir::reexports::IrCmpOp;
        let facts = |func: &IrFunction, slot: &mut Option<bool>| {
            analyze_function_body(
                func,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &FxHashMap::default(),
                &FxHashMap::default(),
                &LibraryFacts::default(),
                slot,
            )
        };
        let counted = function("f", counts(IrCmpOp::Slt, 1));
        let mut slot = None;
        assert!(facts(&counted, &mut slot).returns);
        assert_eq!(slot, Some(true));
        // A remembered answer is not asked again.
        let mut slot = Some(false);
        assert!(!facts(&counted, &mut slot).returns);
        assert_eq!(slot, Some(false));
        // A body that calls something unknown is not asked at all.
        let mut writes = counts(IrCmpOp::Slt, 1);
        writes[2].instructions.push(call("elsewhere"));
        let mut slot = None;
        assert_eq!(facts(&function("f", writes), &mut slot), BodyFacts::UNKNOWN);
        assert_eq!(slot, None);
    }

    /// The out-parameter summary keeps to bodies without a cycle, counted
    /// or not, and without a trap: its callers read it as "the call returns".
    #[test]
    fn a_summary_is_made_only_of_a_body_that_runs_straight_through() {
        use crate::ir::reexports::{IrCmpOp, IrParam};
        let writes_through = |mut blocks: Vec<BasicBlock>, at: usize| {
            blocks[0].instructions.push(Instruction::ParamRef {
                dest: Value(40),
                param_idx: 0,
                ty: IrType::Ptr,
            });
            blocks[at].instructions.push(Instruction::Store {
                val: Operand::Const(IrConst::I32(1)),
                ptr: Value(40),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            let mut func = function("fill", blocks);
            func.params = vec![IrParam {
                ty: IrType::Ptr,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: vec![],
                is_f128_sse: false,
                fp_agg_class: None,
            }];
            analyze_out_param_summary(
                &func,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &FxHashMap::default(),
                &LibraryFacts::default(),
            )
        };
        let straight = vec![block(0, vec![], branch(1)), block(1, vec![], ret(0))];
        let summary = writes_through(straight, 1).expect("a store through the parameter");
        assert_eq!(summary.out_param_mask, 1);
        // The same store in a loop that ends ...
        assert_eq!(writes_through(counts(IrCmpOp::Slt, 1), 2), None);
        // ... and ahead of a branch that may trap.
        let may_trap = vec![
            block(0, vec![], either(1, 2)),
            block(1, vec![], Terminator::Unreachable),
            block(2, vec![], ret(0)),
        ];
        assert_eq!(writes_through(may_trap, 0), None);
    }

    /// Blocks the entry cannot reach describe no execution, whatever they
    /// end in: lowering leaves them behind until the CFG is cleaned.
    #[test]
    fn a_dead_block_decides_nothing() {
        let with_dead_code = vec![
            block(0, vec![], ret(3)),
            block(1, vec![], Terminator::Unreachable),
            block(2, vec![], branch(2)),
        ];
        assert_eq!(purity(with_dead_code), (true, true));
    }

    fn call(callee: &str) -> Instruction {
        Instruction::Call {
            func: callee.into(),
            info: CallInfo {
                return_type: IrType::I32,
                dest: Some(Value(1)),
                ..Default::default()
            },
        }
    }

    fn call_info<'a>(module: &'a IrModule, caller: &str, index: usize) -> &'a CallInfo {
        let caller = module.functions.iter().find(|f| f.name == caller).unwrap();
        match &caller.blocks[0].instructions[index] {
            Instruction::Call { info, .. } => info,
            other => panic!("expected the call, found {other:?}"),
        }
    }

    fn call_flags(module: &IrModule, caller: &str) -> (bool, bool) {
        let info = call_info(module, caller, 0);
        (info.is_pure, info.is_const)
    }

    /// `checks`: `if (c) __builtin_trap(); return 0;`
    fn checks() -> IrFunction {
        function(
            "checks",
            vec![
                block(0, vec![], either(1, 2)),
                block(1, vec![], Terminator::Unreachable),
                block(2, vec![], ret(0)),
            ],
        )
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn param(dest: u32) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: 0,
            ty: IrType::Ptr,
        }
    }

    /// `while (*p) {}`-shaped: reads through its parameter in a loop.
    fn scans() -> IrFunction {
        function(
            "scans",
            vec![
                block(0, vec![param(5)], branch(1)),
                block(1, vec![load(0, 5)], either(1, 2)),
                block(2, vec![], ret(0)),
            ],
        )
    }

    /// The module pass: what DCE and LICM see at the call site.
    #[test]
    fn a_call_to_a_function_that_may_not_return_is_not_marked_removable() {
        let cases: [(&str, Vec<BasicBlock>, (bool, bool)); 3] = [
            (
                "checks",
                vec![
                    block(0, vec![], either(1, 2)),
                    block(1, vec![], Terminator::Unreachable),
                    block(2, vec![], ret(0)),
                ],
                (false, false),
            ),
            (
                "parks",
                vec![
                    block(0, vec![], either(1, 2)),
                    block(1, vec![], branch(1)),
                    block(2, vec![], ret(0)),
                ],
                (false, false),
            ),
            (
                "returns",
                vec![
                    block(0, vec![], either(1, 2)),
                    block(1, vec![], ret(1)),
                    block(2, vec![], ret(2)),
                ],
                (true, true),
            ),
        ];
        for (name, body, expected) in cases {
            let mut module = IrModule::new();
            module.functions.push(function(name, body));
            module
                .functions
                .push(function("caller", vec![block(0, vec![call(name)], ret(0))]));
            run(&mut module);
            assert_eq!(call_flags(&module, "caller"), expected, "{name}");
        }
    }

    /// The property is not inherited upwards by accident: a function whose
    /// only fault is calling one that may not return stays impure, because
    /// that call is.
    #[test]
    fn a_caller_of_a_function_that_may_not_return_is_not_pure_either() {
        let mut module = IrModule::new();
        module.functions.push(function(
            "checks",
            vec![
                block(0, vec![], either(1, 2)),
                block(1, vec![], Terminator::Unreachable),
                block(2, vec![], ret(0)),
            ],
        ));
        module.functions.push(function(
            "relay",
            vec![block(0, vec![call("checks")], ret(0))],
        ));
        module.functions.push(function(
            "caller",
            vec![block(0, vec![call("relay")], ret(0))],
        ));
        run(&mut module);
        assert_eq!(call_flags(&module, "relay"), (false, false));
        assert_eq!(call_flags(&module, "caller"), (false, false));
    }

    /// What such a function leaves alone is still true of it, and is what
    /// lets a second identical call reuse the first: recorded apart from the
    /// flags that would let the call be deleted.
    #[test]
    fn what_a_function_that_may_not_return_leaves_alone_is_recorded() {
        let writes = function(
            "writes",
            vec![block(
                0,
                vec![
                    param(5),
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(1)),
                        ptr: Value(5),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                ],
                Terminator::Unreachable,
            )],
        );
        let cases = [
            (checks(), MemoryEffect::ReadNone),
            (scans(), MemoryEffect::ReadOnly),
            (writes, MemoryEffect::ReadWrite),
        ];
        for (callee, expected) in cases {
            let name = callee.name.clone();
            let mut module = IrModule::new();
            module.functions.push(callee);
            module.functions.push(function(
                "caller",
                vec![block(0, vec![call(&name)], ret(0))],
            ));
            run(&mut module);
            let info = call_info(&module, "caller", 0);
            assert_eq!((info.is_pure, info.is_const), (false, false), "{name}");
            assert_eq!(info.memory, expected, "{name}");
            assert_eq!(info.memory_effect(), expected, "{name}");
        }
    }

    /// A function is what it calls: one that only relays to a callee that may
    /// not return writes as little, and may not return either -- however many
    /// relays deep, and whichever is analysed first.
    #[test]
    fn the_memory_effect_and_the_doubt_both_pass_to_callers() {
        let relay = |name: &str, to: &str| function(name, vec![block(0, vec![call(to)], ret(0))]);
        let mut module = IrModule::new();
        module.functions.push(relay("outer", "inner"));
        module.functions.push(relay("inner", "scans"));
        module.functions.push(scans());
        module.functions.push(relay("top", "outer"));
        run(&mut module);
        for caller in ["inner", "outer", "top"] {
            let info = call_info(&module, caller, 0);
            assert_eq!((info.is_pure, info.is_const), (false, false), "{caller}");
            assert_eq!(info.memory, MemoryEffect::ReadOnly, "{caller}");
        }
    }

    /// A callee that returns is not downgraded by sharing a caller with one
    /// that may not, and the caller is exactly as weak as its weakest call.
    #[test]
    fn a_mixed_caller_keeps_each_call_what_it_is() {
        let returns = function("returns", vec![block(0, vec![], ret(1))]);
        let mut both = call("returns");
        if let Instruction::Call { info, .. } = &mut both {
            info.dest = Some(Value(2));
        }
        let mut module = IrModule::new();
        module.functions.push(returns);
        module.functions.push(checks());
        module.functions.push(function(
            "mixed",
            vec![block(0, vec![call("checks"), both], ret(0))],
        ));
        module
            .functions
            .push(function("top", vec![block(0, vec![call("mixed")], ret(0))]));
        run(&mut module);
        let checked = call_info(&module, "mixed", 0);
        assert_eq!((checked.is_pure, checked.is_const), (false, false));
        assert_eq!(checked.memory, MemoryEffect::ReadNone);
        let returned = call_info(&module, "mixed", 1);
        assert_eq!((returned.is_pure, returned.is_const), (true, true));
        let top = call_info(&module, "top", 0);
        assert_eq!((top.is_pure, top.is_const), (false, false));
        assert_eq!(top.memory, MemoryEffect::ReadNone);
    }

    /// The fact survives the body it was read from: a later run seeds from
    /// the call sites, as it does for `is_pure`.
    #[test]
    fn a_recorded_memory_effect_seeds_the_next_run() {
        let mut declared = function("gone", vec![]);
        declared.is_declaration = true;
        let mut stamped = call("gone");
        if let Instruction::Call { info, .. } = &mut stamped {
            info.memory = MemoryEffect::ReadOnly;
        }
        let mut other = call("gone");
        if let Instruction::Call { info, .. } = &mut other {
            info.dest = Some(Value(2));
        }
        let mut module = IrModule::new();
        module.functions.push(declared);
        module.functions.push(function(
            "caller",
            vec![block(0, vec![stamped, other], ret(0))],
        ));
        module.functions.push(function(
            "top",
            vec![block(0, vec![call("caller")], ret(0))],
        ));
        run(&mut module);
        assert_eq!(
            call_info(&module, "caller", 1).memory,
            MemoryEffect::ReadOnly
        );
        let top = call_info(&module, "top", 0);
        assert_eq!((top.is_pure, top.is_const), (false, false));
        assert_eq!(top.memory, MemoryEffect::ReadOnly);
    }

    /// `__builtin_strlen(s)` is the library function; `strlen(s)` under
    /// `-fno-builtin-strlen` is whatever the program defines.  The first being
    /// pure used to make the second pure on the next run of the pass.
    #[test]
    fn a_library_fact_stays_with_the_calls_that_reach_the_library() {
        let counter = Instruction::GlobalAddr {
            dest: Value(7),
            name: "calls".into(),
        };
        let counts = function(
            "strlen",
            vec![block(
                0,
                vec![
                    counter,
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(1)),
                        ptr: Value(7),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                ],
                ret(0),
            )],
        );
        let site = |library: bool, dest: u32| Instruction::Call {
            func: "strlen".into(),
            info: CallInfo {
                return_type: IrType::I64,
                dest: Some(Value(dest)),
                args: vec![Operand::Value(Value(9))],
                arg_types: vec![IrType::Ptr],
                assume_library: library,
                ..Default::default()
            },
        };
        let mut module = IrModule::new();
        module.functions.push(counts);
        module.functions.push(function(
            "caller",
            vec![block(
                0,
                vec![site(true, 1), site(false, 2), site(false, 3)],
                ret(0),
            )],
        ));
        for round in 0..3 {
            run(&mut module);
            let library = call_info(&module, "caller", 0);
            assert!(library.is_pure && !library.is_const, "round {round}");
            for plain in [1, 2] {
                let info = call_info(&module, "caller", plain);
                assert_eq!(
                    (info.is_pure, info.is_const, info.memory),
                    (false, false, MemoryEffect::ReadWrite),
                    "round {round}, call {plain}"
                );
            }
        }
    }
}
