//! Substitution-based GlobalAddr CSE.
//!
//! The frontend emits one `GlobalAddr` per source-level access, so a loop
//! that touches several file-scope arrays keeps a distinct SSA address value
//! live for every site. Those values are linker constants of the same
//! symbol and are interchangeable. Merging them cuts register pressure
//! (fannkuch's perm/perm1/count each materialized 4–6× on levkropp/lccc).
//!
//! This is **not** GVN Copy-insertion. Pointer-valued Copy chains are the
//! stale-base landmine that keeps GEP CSE disabled; we rewrite uses onto a
//! dominance-safe canonical value and delete the duplicate instruction.
//!
//! Canonical choice is dominance-safe by construction: one materialization
//! of class C is placed in the entry block (reusing an existing one, or
//! inserting after the Alloca/ParamRef prefix). Entry dominates every
//! reachable block, so loop-body and sibling-block duplicates all rewrite
//! onto that value. That is the register-pressure win: fannkuch's
//! perm/perm1/count stop being re-materialized every iteration, and two
//! non-entry blocks that never saw each other still share one SSA web.
//!
//! Class split (the levkropp original mixed these and fought RIP remat):
//! - **Foldable**: every use is a Load/Store pointer, an absorbed GEP/Add/Sub
//!   address producer, or a Copy/same-size Cast of the address. These stay
//!   in `never_materialized` after CSE and become `sym(%rip)` / SIB folds.
//! - **Must-materialize**: any other use (call arg, stored value, inline-asm
//!   operand, intrinsic dest_ptr, return, …). CSE within this class is the
//!   register-pressure win.
//! Mixing the two classes is forbidden: folding a RIP-only address into a
//! value that must occupy a register would pin `window`/`crc_table` in a
//! GPR and undo RA-01.
//!
//! Substitution walks `Instruction::for_each_operand_mut` **and**
//! `for_each_value_use_mut` so Intrinsic `dest_ptr` and InlineAsm outputs
//! cannot be left dangling (the levkropp TCE helper missed both).
//!
//! Kill switches: `CCC_NO_GADDR_CSE`, `CCC_DISABLE_PASSES=gaddrcse`.
//!
//! Scheduling: the pass **must** run before the first GVN. GVN already CSEs
//! `GlobalAddr` (same-block, class-blind Copies). A late-only wiring after
//! post-structural-inline therefore observes `merged=0` on every fannkuch-like
//! shape — the duplicates are already Copies. GVN's GlobalAddr key is also
//! class-split so a later GVN cannot pin a RIP-foldable address into a GPR.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, IrModule, Operand, Value};

fn debug_merged(name: &str, n: usize) {
    if std::env::var_os("CCC_DEBUG_GADDR_CSE").is_some() {
        eprintln!("[GADDR_CSE] fn={} merged={}", name, n);
    }
}

/// Resolve GNU `__attribute__((alias))` chains the same way GVN does, so
/// `GlobalAddr "foo"` and `GlobalAddr "bar"` of an alias pair share a web.
fn alias_canon_map(module: &IrModule) -> FxHashMap<String, String> {
    let direct: FxHashMap<String, String> = module
        .aliases
        .iter()
        .map(|(alias, target, _)| (alias.clone(), target.clone()))
        .collect();
    let mut out = FxHashMap::default();
    for alias in direct.keys() {
        let mut current = alias.as_str();
        let mut seen = FxHashSet::default();
        while let Some(next) = direct.get(current) {
            if !seen.insert(current.to_string()) {
                break;
            }
            current = next;
        }
        out.insert(alias.clone(), current.to_string());
    }
    out
}

fn canon_name<'a>(name: &'a str, aliases: &'a FxHashMap<String, String>) -> &'a str {
    aliases.get(name).map(String::as_str).unwrap_or(name)
}

pub(crate) fn run_module(module: &mut IrModule) -> usize {
    let aliases = alias_canon_map(module);
    module.for_each_function(|f| run_with_aliases(f, &aliases))
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_with_aliases(func, &FxHashMap::default())
}

pub(crate) fn run_with_aliases(
    func: &mut IrFunction,
    aliases: &FxHashMap<String, String>,
) -> usize {
    if func.blocks.is_empty() {
        debug_merged(&func.name, 0);
        return 0;
    }
    if std::env::var_os("CCC_NO_GADDR_CSE").is_some() {
        debug_merged(&func.name, 0);
        return 0;
    }

    let must_mat = classify_must_materialize(func);

    // First existing entry-block GlobalAddr of each (symbol, class) is the
    // canonical dest. Missing classes are inserted after the Alloca/ParamRef
    // prefix so they dominate every reachable use.
    let mut entry_canonical: FxHashMap<(String, bool), Value> = FxHashMap::default();
    for inst in &func.blocks[0].instructions {
        if let Instruction::GlobalAddr { dest, name } = inst {
            let class = must_mat.contains(&dest.0);
            let key = (canon_name(name, aliases).to_string(), class);
            entry_canonical.entry(key).or_insert(*dest);
        }
    }

    let mut needed: FxHashSet<(String, bool)> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, name } = inst {
                let class = must_mat.contains(&dest.0);
                needed.insert((canon_name(name, aliases).to_string(), class));
            }
        }
    }

    let mut to_insert: Vec<(String, bool)> = needed
        .into_iter()
        .filter(|k| !entry_canonical.contains_key(k))
        .collect();
    to_insert.sort();

    if !to_insert.is_empty() {
        let mut next_id = func.next_value_id.max(func.max_value_id().saturating_add(1));
        let insert_at = {
            let mut i = 0usize;
            for inst in &func.blocks[0].instructions {
                match inst {
                    Instruction::Alloca { .. } | Instruction::ParamRef { .. } => i += 1,
                    _ => break,
                }
            }
            i
        };
        let span_lockstep =
            func.blocks[0].source_spans.len() == func.blocks[0].instructions.len();
        let dummy = crate::common::source::Span::dummy();
        for (name, class) in to_insert.iter().rev() {
            let dest = Value(next_id);
            next_id += 1;
            entry_canonical.insert((name.clone(), *class), dest);
            func.blocks[0].instructions.insert(
                insert_at,
                Instruction::GlobalAddr {
                    dest,
                    name: name.clone(),
                },
            );
            if span_lockstep && !func.blocks[0].source_spans.is_empty() {
                func.blocks[0].source_spans.insert(insert_at, dummy);
            }
        }
        func.next_value_id = next_id;
    }

    // Freshly inserted dests are not in `must_mat` (no uses yet). Never
    // treat a chosen canonical as a duplicate of the other class.
    let canon_ids: FxHashSet<u32> = entry_canonical.values().map(|v| v.0).collect();

    let mut subst: FxHashMap<u32, u32> = FxHashMap::default();
    let mut dups: Vec<(usize, usize)> = Vec::new();

    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::GlobalAddr { dest, name } = inst else {
                continue;
            };
            if canon_ids.contains(&dest.0) {
                continue;
            }
            let class = must_mat.contains(&dest.0);
            let key = (canon_name(name, aliases).to_string(), class);
            let Some(&canon) = entry_canonical.get(&key) else {
                continue;
            };
            if canon != *dest {
                subst.insert(dest.0, canon.0);
                dups.push((bi, ii));
            }
        }
    }

    if dups.is_empty() {
        debug_merged(&func.name, 0);
        return 0;
    }

    rewrite_uses(func, &subst);

    let n = dups.len();
    dups.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (bi, ii) in dups {
        let block = &mut func.blocks[bi];
        let span_lockstep = block.source_spans.len() == block.instructions.len();
        block.instructions.remove(ii);
        if span_lockstep {
            block.source_spans.remove(ii);
        } else if !block.source_spans.is_empty() {
            // Desynchronized spans are untrustworthy; drop rather than
            // index a mismatched parallel array (backend -g convention).
            block.source_spans.clear();
        }
    }
    debug_merged(&func.name, n);
    n
}

/// A GlobalAddr dest *must materialize* if any use is not a foldable memory
/// pointer or an absorbed address producer. Conservative: unknown shapes
/// (calls, asm, intrinsics, stored *values*) force materialization.
pub(crate) fn classify_must_materialize(func: &IrFunction) -> FxHashSet<u32> {
    let mut gaddrs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, .. } = inst {
                gaddrs.insert(dest.0);
            }
        }
    }
    if gaddrs.is_empty() {
        return FxHashSet::default();
    }

    // Address producers derived from a GlobalAddr (GEP/Add/Sub/Copy/Cast)
    // that are themselves only used as memory pointers do not force the
    // GlobalAddr to materialize — that is the RIP/SIB fold contract.
    // `parent[dest] = sources` lets a must-materialize use of a derived
    // address (Intrinsic dest_ptr, call arg, …) poison the originating
    // GlobalAddr, not just the derived id.
    let mut derived: FxHashSet<u32> = gaddrs.clone();
    let mut parent: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut link = |dest: u32, src: u32, derived: &mut FxHashSet<u32>,
                    parent: &mut FxHashMap<u32, Vec<u32>>| {
        parent.entry(dest).or_default().push(src);
        derived.insert(dest)
    };
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GetElementPtr { dest, base, .. } if derived.contains(&base.0) => {
                        if link(dest.0, base.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } if derived.contains(&src.0) => {
                        if link(dest.0, src.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(src),
                        from_ty,
                        to_ty,
                        ..
                    } if derived.contains(&src.0)
                        && from_ty.size() == to_ty.size()
                        && !from_ty.is_float()
                        && !to_ty.is_float() =>
                    {
                        if link(dest.0, src.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub,
                        lhs,
                        rhs,
                        ..
                    } => {
                        if let Operand::Value(v) = lhs {
                            if derived.contains(&v.0)
                                && link(dest.0, v.0, &mut derived, &mut parent)
                            {
                                changed = true;
                            }
                        }
                        if let Operand::Value(v) = rhs {
                            if derived.contains(&v.0)
                                && link(dest.0, v.0, &mut derived, &mut parent)
                            {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut must = FxHashSet::default();
    let mut mark = |id: u32| {
        let mut stack = vec![id];
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if gaddrs.contains(&cur) {
                must.insert(cur);
            }
            if let Some(ps) = parent.get(&cur) {
                stack.extend(ps.iter().copied());
            }
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { ptr, .. } => {
                    // Pointer use of a derived address is foldable.
                    let _ = ptr;
                }
                Instruction::Store { val, ptr, .. } => {
                    if let Operand::Value(v) = val {
                        mark(v.0);
                    }
                    let _ = ptr;
                }
                Instruction::GetElementPtr { .. }
                | Instruction::Copy { .. }
                | Instruction::Cast { .. }
                | Instruction::BinOp {
                    op: crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub,
                    ..
                } => {}
                _ => {
                    inst.for_each_used_value(|id| mark(id));
                }
            }
        }
        block.terminator.for_each_used_value(|id| mark(id));
    }
    must
}

fn rewrite_uses(func: &mut IrFunction, subst: &FxHashMap<u32, u32>) {
    let rewrite_val = |v: &mut Value| {
        if let Some(&to) = subst.get(&v.0) {
            *v = Value(to);
        }
    };
    let rewrite_op = |op: &mut Operand| {
        if let Operand::Value(v) = op {
            rewrite_val(v);
        }
    };
    for block in func.blocks.iter_mut() {
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(rewrite_op);
            inst.for_each_value_use_mut(rewrite_val);
        }
        block.terminator.for_each_operand_mut(rewrite_op);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Operand, Terminator};

    fn empty_func() -> IrFunction {
        let mut f = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        f.next_value_id = 10;
        f
    }

    fn store(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(0)),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
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

    #[test]
    fn merges_same_symbol_in_block_foldable() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[0].instructions.len(), 2);
        match &func.blocks[0].instructions[1] {
            Instruction::Store { ptr, .. } => assert_eq!(ptr.0, 1),
            _ => panic!("expected store"),
        }
    }

    #[test]
    fn entry_block_canonical_wins_cross_block() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::GlobalAddr {
                dest: Value(1),
                name: "g".to_string(),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[1].instructions.len(), 1);
        match &func.blocks[1].instructions[0] {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 1),
            _ => panic!("expected load"),
        }
    }

    #[test]
    fn distinct_symbols_untouched() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "a".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "b".to_string(),
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 2);
    }

    #[test]
    fn does_not_mix_foldable_with_must_materialize() {
        // v1 is only a load pointer (foldable). v2 is passed to a call
        // (must materialize). CSE must not merge them: that would pin the
        // RIP-foldable address in a register.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                load(3, 1),
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                Instruction::Call {
                    func: "use_ptr".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(2))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::Void,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: vec![],
                        struct_arg_riscv_float_classes: vec![],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 4);
    }

    #[test]
    fn rewrites_intrinsic_dest_ptr() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "buf".to_string(),
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                    dest_ptr: Some(Value(1)),
                    args: vec![Operand::Value(Value(8))],
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "buf".to_string(),
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                    dest_ptr: Some(Value(2)),
                    args: vec![Operand::Value(Value(9))],
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[0].instructions.len(), 3);
        match &func.blocks[0].instructions[2] {
            Instruction::Intrinsic {
                dest_ptr: Some(p), ..
            } => assert_eq!(p.0, 1),
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn keeps_source_spans_lockstep() {
        let mut func = empty_func();
        let dummy = crate::common::source::Span::dummy();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![dummy, dummy, dummy],
        });
        run(&mut func);
        assert_eq!(
            func.blocks[0].source_spans.len(),
            func.blocks[0].instructions.len()
        );
    }

    #[test]
    fn hoists_non_entry_to_entry() {
        // A single loop-body GlobalAddr must move to entry so LICM/RA see
        // one dominating def rather than a per-iteration materialization.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        match &func.blocks[0].instructions[0] {
            Instruction::GlobalAddr { dest, name } => {
                assert_eq!(name, "g");
                assert_eq!(func.blocks[1].instructions.len(), 1);
                match &func.blocks[1].instructions[0] {
                    Instruction::Load { ptr, .. } => assert_eq!(ptr.0, dest.0),
                    other => panic!("expected load, got {other:?}"),
                }
            }
            other => panic!("expected entry GlobalAddr, got {other:?}"),
        }
    }

    #[test]
    fn two_non_entry_blocks_share_entry_canonical() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "g".to_string(),
                },
                load(8, 7),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 2);
        let Instruction::GlobalAddr { dest: canon, .. } = &func.blocks[0].instructions[0] else {
            panic!("expected entry GlobalAddr");
        };
        match &func.blocks[1].instructions[0] {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, canon.0),
            other => panic!("expected load, got {other:?}"),
        }
        match &func.blocks[2].instructions[0] {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, canon.0),
            other => panic!("expected load, got {other:?}"),
        }
    }

    #[test]
    fn aliases_share_a_web() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "foo".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "bar".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let mut aliases = FxHashMap::default();
        aliases.insert("bar".to_string(), "foo".to_string());
        let n = run_with_aliases(&mut func, &aliases);
        assert_eq!(n, 1);
        match &func.blocks[0].instructions[1] {
            Instruction::Store { ptr, .. } => assert_eq!(ptr.0, 1),
            other => panic!("expected store, got {other:?}"),
        }
    }

    #[test]
    fn hoist_preserves_class_split() {
        // Foldable load in block 1 and call-arg in block 2 of the same
        // symbol must become TWO entry GlobalAddrs, never one.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "g".to_string(),
                },
                Instruction::Call {
                    func: "use_ptr".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(7))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::Void,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: vec![],
                        struct_arg_riscv_float_classes: vec![],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 2);
        let entry_gaddrs: Vec<_> = func.blocks[0]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::GlobalAddr { .. }))
            .collect();
        assert_eq!(entry_gaddrs.len(), 2);
    }
}
