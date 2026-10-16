//! Data-placement policy; no instruction gains an aligned-access assumption.
use super::common::PtrDirective;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IntrinsicOp, IrBinOp, IrGlobal, IrModule, Operand};

pub(super) struct DataAlignment {
    optimize_size: bool,
    wide_globals: FxHashSet<String>,
}

impl DataAlignment {
    pub(super) fn new(module: &IrModule, ptr_dir: PtrDirective, optimize_size: bool) -> Self {
        let mut wide_globals = FxHashSet::default();
        // .quad identifies x86-64 here; AArch64/RV64 use .xword/.dword.
        if !optimize_size && matches!(ptr_dir, PtrDirective::Quad) {
            for function in &module.functions {
                let mut pending = Vec::new();
                for inst in function.blocks.iter().flat_map(|b| &b.instructions) {
                    enqueue_wide_addresses(inst, &mut pending);
                }
                if pending.is_empty() {
                    continue;
                }
                // Data emission may run AFTER phi destruction. A value can
                // have both its entry copy and backedge copy: keeping only
                // the last definition loses the global behind a recurrence.
                // Taking their union only affects placement, never an alias
                // or aligned-load assumption.
                let mut definitions: FxHashMap<u32, Vec<&Instruction>> = FxHashMap::default();
                for inst in function.blocks.iter().flat_map(|b| &b.instructions) {
                    if let Some(dest) = inst.dest() {
                        definitions.entry(dest.0).or_default().push(inst);
                    }
                }
                let mut seen = FxHashSet::default();
                while let Some(operand) = pending.pop() {
                    let Operand::Value(value) = operand else {
                        continue;
                    };
                    if !seen.insert(value.0) {
                        continue;
                    }
                    let Some(sites) = definitions.get(&value.0) else {
                        continue;
                    };
                    for instruction in sites {
                        match instruction {
                            Instruction::GlobalAddr { name, .. } => {
                                wide_globals.insert(name.clone());
                            }
                            Instruction::GetElementPtr { base, .. } => {
                                pending.push(Operand::Value(*base))
                            }
                            Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => {
                                pending.push(*src)
                            }
                            Instruction::Phi { incoming, .. } => {
                                pending.extend(incoming.iter().map(|(op, _)| *op))
                            }
                            Instruction::Select {
                                true_val,
                                false_val,
                                ..
                            } => pending.extend([*true_val, *false_val]),
                            Instruction::BinOp {
                                op: IrBinOp::Add | IrBinOp::Sub,
                                lhs,
                                rhs,
                                ..
                            } => pending.extend([*lhs, *rhs]),
                            _ => {}
                        }
                    }
                }
            }
        }
        Self {
            optimize_size,
            wide_globals,
        }
    }

    /// Keep natural and explicit ABI/layout requirements. The old automatic
    /// 16-byte policy remains the default; only traced 256-bit accesses request
    /// 32-byte placement. Do not pad custom-section tables, TLS or size builds.
    pub(super) fn effective(&self, global: &IrGlobal) -> usize {
        if self.optimize_size || global.section.is_some() || global.has_explicit_align {
            return global.align;
        }
        let minimum = if global.size >= 32
            && !global.is_thread_local
            && self.wide_globals.contains(&global.name)
        {
            32
        } else if global.size >= 16 {
            16
        } else {
            0
        };
        global.align.max(minimum)
    }
}

// Operand roles are explicit: a SIMD home width does not establish an access
// size, and the scalar broadcast/offset operands must not be treated as vectors.
fn enqueue_wide_addresses(inst: &Instruction, pending: &mut Vec<Operand>) {
    let Instruction::Intrinsic {
        op, args, dest_ptr, ..
    } = inst
    else {
        return;
    };
    use IntrinsicOp::*;
    match op {
        VecLoadF64x4 | VecLoadF32x8 | VecLoadI32x8 | VecLoadI64x4 | VecLoadI8x32
        | VecLoadI16x16 | LoadF64x4 | LoadI32x8 => pending.extend(args.first().copied()),
        VecStoreF64x4 | VecStoreF32x8 | VecStoreI32x8 | VecStoreI64x4 | VecStoreI8x32
        | VecStoreI16x16 | FmaF64x4 => pending.extend(args.get(1).copied()),
        FmaF64x4Hoisted => pending.extend(args.first().copied()),
        FmaF64x4SIB => pending.extend(args.iter().skip(1).take(2).copied()),
        FmaF64x4HoistedSIB | AddF64x4 | MulF64x4 | AddI32x8 => {
            pending.extend(args.iter().take(2).copied())
        }
        _ => return,
    }
    if let Some(pointer) = dest_ptr {
        pending.push(Operand::Value(*pointer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{
        BasicBlock, BlockId, GlobalInit, IrConst, IrFunction, Terminator, Value,
    };

    // Graph-only analysis fixture: the synthetic cyclic phi deliberately tests
    // termination without requiring executable control flow or vector values.
    fn fixture(op: IntrinsicOp) -> IrModule {
        let mut module = IrModule::new();
        module.globals.push(IrGlobal {
            name: "wide".into(),
            ty: IrType::F64,
            size: 512,
            align: 8,
            init: GlobalInit::Zero,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: false,
            is_used: false,
            is_thread_local: false,
        });
        let mut function = IrFunction::new("vector_user".into(), IrType::Void, vec![], false);
        function.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            terminator: Terminator::Return(None),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "wide".into(),
                },
                Instruction::GetElementPtr {
                    dest: Value(1),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(16)),
                    ty: IrType::F64,
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                },
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::Ptr,
                    incoming: vec![
                        (Operand::Value(Value(2)), BlockId(0)),
                        (Operand::Value(Value(4)), BlockId(0)),
                    ],
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Add,
                    ty: IrType::Ptr,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I64(32)),
                },
                Instruction::Intrinsic {
                    dest: Some(Value(5)),
                    op,
                    dest_ptr: None,
                    args: vec![Operand::Value(Value(3)), Operand::Const(IrConst::I64(0))],
                },
            ],
        });
        module.functions.push(function);
        module
    }

    #[test]
    fn traces_wide_access_through_gep_and_cyclic_phi_without_alignment_claim() {
        let module = fixture(IntrinsicOp::VecLoadF64x4);
        let policy = DataAlignment::new(&module, PtrDirective::Quad, false);
        assert_eq!(policy.effective(&module.globals[0]), 32);
        // A nonzero offset was deliberately retained: better placement does
        // not prove that the individual vector access is aligned.
        assert_eq!(module.globals[0].align, 8);
    }

    #[test]
    fn recognizes_all_register_vector_loads_and_args_only_store() {
        for op in [
            IntrinsicOp::VecLoadF64x4,
            IntrinsicOp::VecLoadF32x8,
            IntrinsicOp::VecLoadI32x8,
            IntrinsicOp::VecLoadI64x4,
            IntrinsicOp::VecLoadI8x32,
            IntrinsicOp::VecLoadI16x16,
        ] {
            let module = fixture(op);
            assert_eq!(
                DataAlignment::new(&module, PtrDirective::Quad, false)
                    .effective(&module.globals[0]),
                32
            );
        }
        let mut module = fixture(IntrinsicOp::VecZeroF64x4);
        if let Instruction::Intrinsic { args, .. } = module.functions[0].blocks[0]
            .instructions
            .last_mut()
            .unwrap()
        {
            args.clear();
        }
        assert_eq!(
            DataAlignment::new(&module, PtrDirective::Quad, false).effective(&module.globals[0]),
            16
        );
        module.functions[0].blocks[0]
            .instructions
            .push(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreF64x4,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(5)),
                    Operand::Value(Value(3)),
                    Operand::Const(IrConst::I64(0)),
                ],
            });
        assert_eq!(
            DataAlignment::new(&module, PtrDirective::Quad, false).effective(&module.globals[0]),
            32
        );
    }

    #[test]
    fn retains_entry_roots_after_phi_destruction() {
        let mut module = fixture(IntrinsicOp::VecLoadF64x4);
        let instructions = &mut module.functions[0].blocks[0].instructions;
        instructions[3] = Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(2)),
        };
        instructions.push(Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(4)),
        });
        assert_eq!(
            DataAlignment::new(&module, PtrDirective::Quad, false).effective(&module.globals[0]),
            32
        );
    }

    #[test]
    fn legacy_fma_roles_exclude_scalar_broadcast_and_index() {
        let pointer = |n| Operand::Value(Value(n));
        for (op, args, dest_ptr) in [
            (
                IntrinsicOp::FmaF64x4,
                vec![pointer(6), pointer(7)],
                Some(Value(3)),
            ),
            (
                IntrinsicOp::FmaF64x4Hoisted,
                vec![pointer(7)],
                Some(Value(3)),
            ),
            (
                IntrinsicOp::FmaF64x4SIB,
                vec![pointer(6), pointer(3), pointer(7), pointer(6)],
                None,
            ),
            (
                IntrinsicOp::FmaF64x4HoistedSIB,
                vec![pointer(3), pointer(7), pointer(6)],
                None,
            ),
        ] {
            let mut module = fixture(IntrinsicOp::VecZeroF64x4);
            for name in ["scalar", "source"] {
                let mut global = module.globals[0].clone();
                global.name = name.into();
                module.globals.push(global);
            }
            let instructions = &mut module.functions[0].blocks[0].instructions;
            instructions.pop();
            instructions.extend([
                Instruction::GlobalAddr {
                    dest: Value(6),
                    name: "scalar".into(),
                },
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "source".into(),
                },
                Instruction::Intrinsic {
                    dest: None,
                    op,
                    dest_ptr,
                    args,
                },
            ]);
            let policy = DataAlignment::new(&module, PtrDirective::Quad, false);
            assert_eq!(policy.effective(&module.globals[0]), 32);
            assert_eq!(policy.effective(&module.globals[1]), 16);
            assert_eq!(policy.effective(&module.globals[2]), 32);
        }
    }

    #[test]
    fn narrow_other_targets_and_layout_contracts_keep_original_policy() {
        let mut module = fixture(IntrinsicOp::VecLoadF64x2);
        assert_eq!(
            DataAlignment::new(&module, PtrDirective::Quad, false).effective(&module.globals[0]),
            16
        );
        module = fixture(IntrinsicOp::VecLoadF64x4);
        for target in [PtrDirective::Long, PtrDirective::Xword, PtrDirective::Dword] {
            assert_eq!(
                DataAlignment::new(&module, target, false).effective(&module.globals[0]),
                16
            );
        }
        assert_eq!(
            DataAlignment::new(&module, PtrDirective::Quad, true).effective(&module.globals[0]),
            8
        );
        let policy = DataAlignment::new(&module, PtrDirective::Quad, false);
        let mut global = module.globals[0].clone();
        global.has_explicit_align = true;
        assert_eq!(policy.effective(&global), 8);
        global.has_explicit_align = false;
        global.section = Some(".records".into());
        assert_eq!(policy.effective(&global), 8);
        global.section = None;
        global.is_thread_local = true;
        assert_eq!(policy.effective(&global), 16);
        global.is_thread_local = false;
        global.align = 64;
        assert_eq!(policy.effective(&global), 64);
        global.size = 8;
        global.align = 8;
        assert_eq!(policy.effective(&global), 8);
    }
}
