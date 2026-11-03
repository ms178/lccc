//! Late Microsoft integer128 argument transport. Semantic integer IR is kept
//! through optimization/inlining; only the backend view receives pointers to
//! private, 16-byte-aligned argument images. No semantic caller/callee type
//! mismatch is exposed to an optimizer.
use std::borrow::Cow;

use crate::common::source::Span;
use crate::common::types::{AddressSpace, CallingConvention, IrType};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

fn wide(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

fn fresh(next: &mut u32) -> Value {
    let value = Value(*next);
    *next += 1;
    value
}

fn alloca(dest: Value, ty: IrType, size: usize, align: usize) -> Instruction {
    Instruction::Alloca {
        dest,
        ty,
        size,
        align,
        volatile: false,
        semantic_volatile: false,
    }
}

fn load(dest: Value, ptr: Value, ty: IrType) -> Instruction {
    Instruction::Load {
        dest,
        ptr,
        ty,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

fn store(val: Operand, ptr: Value, ty: IrType) -> Instruction {
    Instruction::Store {
        val,
        ptr,
        ty,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

/// Return an untouched borrowed function for the overwhelmingly common case.
/// Physical copies are made after optimization, once per call, and cannot alias
/// the caller's language-level object or a live SSA integer128 image.
pub(super) fn legalize(func: &IrFunction) -> Cow<'_, IrFunction> {
    let ms = CallingConvention::MicrosoftX64;
    let params: Vec<_> = if func.calling_convention == ms {
        func.params
            .iter()
            .enumerate()
            .filter_map(|(i, p)| wide(p.ty).then_some((i, p.ty)))
            .collect()
    } else {
        Vec::new()
    };
    let needs_calls = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|inst| {
            matches!(inst, Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. }
            if info.calling_convention == ms && info.arg_types.iter().copied().any(wide))
        });
    if func.is_declaration || (params.is_empty() && !needs_calls) {
        return Cow::Borrowed(func);
    }
    let mut out = func.clone();
    let mut next = out.sound_next_value_id();
    let mut entry = Vec::new();
    let mut snapshots = Vec::new();
    for (idx, ty) in params {
        let physical_home = fresh(&mut next);
        let pointer = fresh(&mut next);
        let snapshot = fresh(&mut next);
        entry.push((alloca(physical_home, IrType::Ptr, 8, 8), Span::dummy()));
        entry.push((
            Instruction::ParamRef {
                dest: pointer,
                param_idx: idx,
                ty: IrType::Ptr,
            },
            Span::dummy(),
        ));
        entry.push((load(snapshot, pointer, ty), Span::dummy()));
        // An unpromoted parameter object still needs its initial value. Move
        // its alloca before the store, rather than creating a use before def.
        if let Some(old_home) = out.param_alloca_values.get(idx).copied() {
            for block in &mut out.blocks {
                if let Some(pos) = block.instructions.iter().position(|inst| {
                    matches!(inst,
                    Instruction::Alloca { dest, .. } if *dest == old_home)
                }) {
                    let inst = block.instructions.remove(pos);
                    let span = if pos < block.source_spans.len() {
                        block.source_spans.remove(pos)
                    } else {
                        Span::dummy()
                    };
                    entry.push((inst, span));
                    entry.push((store(Operand::Value(snapshot), old_home, ty), span));
                    break;
                }
            }
            out.param_alloca_values[idx] = physical_home;
        }
        out.params[idx].ty = IrType::Ptr;
        snapshots.push((idx, snapshot));
    }
    for block in &mut out.blocks {
        let original = std::mem::take(&mut block.instructions);
        let spans = std::mem::take(&mut block.source_spans);
        for (index, mut inst) in original.into_iter().enumerate() {
            let span = spans.get(index).copied().unwrap_or_else(Span::dummy);
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = &inst
            {
                if let Some((_, value)) = snapshots.iter().find(|(idx, _)| idx == param_idx) {
                    inst = Instruction::Copy {
                        dest: *dest,
                        src: Operand::Value(*value),
                    };
                }
            }
            if let Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } =
                &mut inst
            {
                if info.calling_convention == ms {
                    for i in 0..info.args.len() {
                        let ty = info.arg_types[i];
                        if wide(ty) {
                            let copy = fresh(&mut next);
                            block.instructions.push(alloca(copy, ty, 16, 16));
                            block.source_spans.push(span);
                            block
                                .instructions
                                .push(store(info.args[i].clone(), copy, ty));
                            block.source_spans.push(span);
                            info.args[i] = Operand::Value(copy);
                            info.arg_types[i] = IrType::Ptr;
                            if let Some(flag) = info.struct_arg_is_f128_sse.get_mut(i) {
                                *flag = false;
                            }
                        }
                    }
                }
            }
            block.instructions.push(inst);
            block.source_spans.push(span);
        }
    }
    if let Some(first) = out.blocks.first_mut() {
        let (mut instructions, mut spans): (Vec<_>, Vec<_>) = entry.into_iter().unzip();
        instructions.append(&mut first.instructions);
        spans.append(&mut first.source_spans);
        first.instructions = instructions;
        first.source_spans = spans;
    }
    out.next_value_id = next;
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, IrParam, Terminator};

    fn fixture() -> IrFunction {
        let mut f = IrFunction::new(
            "wide".into(),
            IrType::I128,
            vec![IrParam {
                ty: IrType::I128,
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
        f.calling_convention = CallingConvention::MicrosoftX64;
        f.param_alloca_values = vec![Value(9)];
        f.next_value_id = 1; // Deliberately stale: allocation must scan uses too.
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            instructions: vec![
                alloca(Value(9), IrType::I128, 16, 16),
                Instruction::ParamRef {
                    dest: Value(90),
                    param_idx: 0,
                    ty: IrType::I128,
                },
                Instruction::Call {
                    func: "other".into(),
                    info: CallInfo {
                        calling_convention: CallingConvention::MicrosoftX64,
                        args: vec![Operand::Value(Value(90))],
                        arg_types: vec![IrType::I128],
                        ..Default::default()
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(90)))),
        });
        f
    }

    #[test]
    fn unrelated_functions_are_borrowed_without_copying() {
        let f = IrFunction::new("scalar".into(), IrType::I64, vec![], false);
        assert!(matches!(legalize(&f), Cow::Borrowed(_)));
    }

    #[test]
    fn physical_view_preserves_semantic_ir_spans_and_unique_ids() {
        let f = fixture();
        let physical = legalize(&f);
        assert!(matches!(physical, Cow::Owned(_)));
        assert_eq!(f.params[0].ty, IrType::I128);
        assert_eq!(f.blocks[0].instructions.len(), 3);
        assert_eq!(physical.params[0].ty, IrType::Ptr);
        assert!(physical.next_value_id > 90);
        let block = &physical.blocks[0];
        assert!(block.spans_parallel());
        let mut definitions = std::collections::HashSet::new();
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                assert!(definitions.insert(dest.0));
            }
            if let Instruction::Call { info, .. } = inst {
                assert_eq!(info.arg_types, vec![IrType::Ptr]);
                assert!(!matches!(info.args[0], Operand::Value(Value(90))));
            }
        }
        let alloca_pos = block
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Alloca { dest: Value(9), .. }))
            .unwrap();
        let store_pos = block
            .instructions
            .iter()
            .position(|i| {
                matches!(
                    i,
                    Instruction::Store {
                        ptr: Value(9),
                        ty: IrType::I128,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(alloca_pos < store_pos);
        assert!(matches!(legalize(&physical), Cow::Borrowed(_)));
    }

    #[test]
    fn sysv_call_operands_are_not_reinterpreted() {
        let mut f = fixture();
        if let Instruction::Call { info, .. } = &mut f.blocks[0].instructions[2] {
            info.calling_convention = CallingConvention::Default;
        }
        let physical = legalize(&f);
        for inst in &physical.blocks[0].instructions {
            if let Instruction::Call { info, .. } = inst {
                assert_eq!(info.arg_types, vec![IrType::I128]);
                assert!(matches!(info.args[0], Operand::Value(Value(90))));
            }
        }
    }
}
