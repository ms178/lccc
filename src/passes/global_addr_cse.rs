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
//! Canonical choice is dominance-safe by construction:
//! - an entry-block materialization of class C dominates every reachable
//!   block, so it is canonical for every later same-symbol, same-class
//!   duplicate;
//! - otherwise the first materialization of class C within a block is
//!   canonical for later same-block duplicates.
//!
//! Cross-block merging between two non-entry blocks is not attempted.
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

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

pub(crate) fn run(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_GADDR_CSE").is_ok() || func.blocks.is_empty() {
        return 0;
    }

    let must_mat = classify_must_materialize(func);

    // Entry-block canonicals keyed by (symbol, must_materialize).
    let mut entry_canonical: FxHashMap<(String, bool), Value> = FxHashMap::default();
    for inst in &func.blocks[0].instructions {
        if let Instruction::GlobalAddr { dest, name } = inst {
            let class = must_mat.contains(&dest.0);
            entry_canonical
                .entry((name.clone(), class))
                .or_insert(*dest);
        }
    }

    let mut subst: FxHashMap<u32, u32> = FxHashMap::default();
    let mut dups: Vec<(usize, usize)> = Vec::new();

    for (bi, block) in func.blocks.iter().enumerate() {
        let mut block_canonical: FxHashMap<(String, bool), Value> = FxHashMap::default();
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::GlobalAddr { dest, name } = inst else {
                continue;
            };
            let class = must_mat.contains(&dest.0);
            let key = (name.clone(), class);
            if let Some(&canon) = entry_canonical.get(&key) {
                if canon != *dest {
                    subst.insert(dest.0, canon.0);
                    dups.push((bi, ii));
                }
                continue;
            }
            match block_canonical.get(&key) {
                Some(&canon) => {
                    subst.insert(dest.0, canon.0);
                    dups.push((bi, ii));
                }
                None => {
                    block_canonical.insert(key, *dest);
                }
            }
        }
    }

    if dups.is_empty() {
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
    n
}

/// A GlobalAddr dest *must materialize* if any use is not a foldable memory
/// pointer or an absorbed address producer. Conservative: unknown shapes
/// (calls, asm, intrinsics, stored *values*) force materialization.
fn classify_must_materialize(func: &IrFunction) -> FxHashSet<u32> {
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
}
