use super::*;

fn evaluate(insts: &[Instruction], result: Operand, inputs: [u64; 3], width: usize) -> u64 {
    let mask = u64::MAX >> (64 - width);
    let mut values = [0u64; 128];
    values[..3].copy_from_slice(&inputs);
    let get = |op: Operand, values: &[u64]| match op {
        Operand::Value(v) => values[v.0 as usize] & mask,
        _ => const_in_ring(op, width).unwrap(),
    };
    for inst in insts {
        let (dest, value) = match inst {
            Instruction::Copy { dest, src } => (*dest, get(*src, &values)),
            Instruction::UnaryOp {
                dest,
                op: IrUnaryOp::Not,
                src,
                ..
            } => (*dest, !get(*src, &values)),
            Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } => {
                let (a, b) = (get(*lhs, &values), get(*rhs, &values));
                (
                    *dest,
                    match op {
                        IrBinOp::And => a & b,
                        IrBinOp::Or => a | b,
                        IrBinOp::Xor => a ^ b,
                        IrBinOp::Add => a.wrapping_add(b),
                        _ => panic!("unexpected operation"),
                    },
                )
            }
            _ => panic!("unexpected instruction"),
        };
        values[dest.0 as usize] = value & mask;
    }
    get(result, &values)
}
fn rewrite(insts: &[Instruction], shared: Option<Value>) -> Option<Vec<Instruction>> {
    rewrite_with_policy(insts, shared, false)
}
fn rewrite_with_policy(
    insts: &[Instruction],
    shared: Option<Value>,
    disable_logic: bool,
) -> Option<Vec<Instruction>> {
    let mut defs = vec![None; 128];
    let mut uses = vec![0u32; 128];
    for inst in insts {
        defs[inst.dest()?.0 as usize] = Some(inst.clone());
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(v) = op {
                uses[v.0 as usize] += 1;
            }
        });
    }
    if let Some(v) = shared {
        uses[v.0 as usize] += 1;
    }
    fold(
        insts.last()?,
        &Context {
            defs: &defs,
            uses: &uses,
            visits: std::cell::Cell::new(512),
            disable_logic,
            wrap_signed_overflow: true,
        },
        Value(80),
    )
}
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

#[test]
fn every_recipe_and_network_matches_all_bit_valuations_and_word_widths() {
    let mut admitted = 0;
    for (bits, recipe) in recipes().iter().enumerate() {
        if recipe.is_none() {
            continue;
        }
        admitted += 1;
        for ty in [
            IrType::U8,
            IrType::I8,
            IrType::U16,
            IrType::I16,
            IrType::U32,
            IrType::I32,
            IrType::U64,
            IrType::I64,
        ] {
            let width = ty.size() * 8;
            let mask = u64::MAX >> (64 - width);
            let mut source = Vec::new();
            let base = emit(bits as u8, &[v(0), v(1), v(2)], ty, Value(3), &mut source).unwrap();
            assert_eq!(source.len(), recipe.unwrap().cost as usize);
            source.push(Instruction::UnaryOp {
                dest: Value(40),
                op: IrUnaryOp::Not,
                src: base,
                ty,
            });
            source.push(Instruction::UnaryOp {
                dest: Value(41),
                op: IrUnaryOp::Not,
                src: v(40),
                ty,
            });
            let replacement = rewrite(&source, None).unwrap();
            assert!(replacement.len() < source.len());
            for valuation in 0..8 {
                let inputs =
                    std::array::from_fn(|i| if valuation & (1 << i) != 0 { mask } else { 0 });
                let expected = if bits & (1 << valuation) != 0 {
                    mask
                } else {
                    0
                };
                assert_eq!(evaluate(&source, v(41), inputs, width), expected);
                assert_eq!(evaluate(&replacement, v(41), inputs, width), expected);
            }
            let mut seed = 0x243f6a8885a308d3u64;
            for _ in 0..32 {
                let inputs = std::array::from_fn(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed & mask
                });
                let expected = (0..width).fold(0, |acc, bit| {
                    let valuation = ((inputs[0] >> bit) & 1)
                        | (((inputs[1] >> bit) & 1) << 1)
                        | (((inputs[2] >> bit) & 1) << 2);
                    acc | ((((bits as u64) >> valuation) & 1) << bit)
                });
                assert_eq!(evaluate(&replacement, v(41), inputs, width), expected);
            }
        }
    }
    assert!(
        admitted >= 50,
        "at least fifty distinct truth functions exercised"
    );
}

fn hidden_xor(ty: IrType) -> Vec<Instruction> {
    vec![
        bin(Value(3), IrBinOp::Or, v(0), v(1), ty),
        bin(Value(4), IrBinOp::And, v(0), v(1), ty),
        Instruction::UnaryOp {
            dest: Value(5),
            op: IrUnaryOp::Not,
            src: v(4),
            ty,
        },
        bin(Value(6), IrBinOp::And, v(3), v(5), ty),
    ]
}
#[test]
fn hidden_xor_reduces_but_shared_subtrees_do_not_pay_for_new_work() {
    let source = hidden_xor(IrType::U32);
    let replacement = rewrite(&source, None).unwrap();
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::BinOp {
            op: IrBinOp::Xor,
            ..
        }]
    ));
    assert!(rewrite(&source, Some(Value(4))).is_none());
}
#[test]
fn complementary_literal_masks_share_one_truth_column() {
    let ty = IrType::U64;
    let mask = 0xff00ff00aa55aa55;
    let source = vec![
        bin(Value(3), IrBinOp::And, v(0), constant(!mask, ty), ty),
        bin(Value(4), IrBinOp::And, v(0), constant(mask, ty), ty),
        bin(Value(5), IrBinOp::And, v(1), constant(mask, ty), ty),
        Instruction::UnaryOp {
            dest: Value(6),
            op: IrUnaryOp::Not,
            src: v(5),
            ty,
        },
        bin(Value(7), IrBinOp::And, v(4), v(6), ty),
        bin(Value(8), IrBinOp::Or, v(3), v(7), ty),
    ];
    let replacement = rewrite(&source, None).unwrap();
    assert!(replacement.len() < source.len());
    for inputs in [
        [0, 0, 0],
        [u64::MAX, u64::MAX, 0],
        [0x123456789abcdef0, 0xfedcba9876543210, 0],
    ] {
        assert_eq!(
            evaluate(&replacement, v(8), inputs, 64),
            inputs[0] & !(inputs[1] & mask)
        );
    }
}
#[test]
fn leaf_budget_walk_budget_and_equal_cost_are_refusals() {
    let ty = IrType::U32;
    let source = vec![
        bin(Value(4), IrBinOp::And, v(0), v(1), ty),
        bin(Value(5), IrBinOp::And, v(2), v(3), ty),
        bin(Value(6), IrBinOp::Or, v(4), v(5), ty),
    ];
    assert!(rewrite(&source, None).is_none());
    assert!(rewrite(&[bin(Value(3), IrBinOp::Xor, v(0), v(1), ty)], None).is_none());
    let mut deep = Vec::new();
    for n in 3..40 {
        deep.push(Instruction::UnaryOp {
            dest: Value(n),
            op: IrUnaryOp::Not,
            src: if n == 3 { v(0) } else { v(n - 1) },
            ty,
        });
    }
    assert!(rewrite(&deep, None).is_none());
    assert!(rewrite(&[bin(Value(3), IrBinOp::Add, v(0), v(1), ty)], None).is_none());
    assert!(rewrite(&hidden_xor(IrType::F32), None).is_none());
}

#[test]
fn internally_shared_producers_are_charged_once() {
    let ty = IrType::U32;
    let source = vec![
        bin(Value(3), IrBinOp::And, v(0), v(1), ty),
        bin(Value(4), IrBinOp::Xor, v(3), v(1), ty),
        bin(Value(5), IrBinOp::Xor, v(3), v(0), ty),
        bin(Value(6), IrBinOp::Or, v(4), v(5), ty),
    ];
    let replacement = rewrite(&source, None).unwrap();
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::BinOp {
            op: IrBinOp::Xor,
            ..
        }]
    ));
    assert!(rewrite(&source, Some(Value(3))).is_none());
}

#[test]
fn owner_counts_bare_pointer_uses_before_charging_a_closed_dag() {
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    for bare_use in [false, true] {
        let mut instructions = hidden_xor(IrType::U64);
        if bare_use {
            instructions.push(Instruction::StackRestore { ptr: Value(4) });
        }
        let mut function = IrFunction::new("bare_use".into(), IrType::U64, vec![], false);
        function.blocks = vec![BasicBlock {
            label: BlockId(0),
            source_spans: vec![Span::dummy(); instructions.len()],
            instructions,
            terminator: Terminator::Return(Some(v(6))),
        }];
        function.next_value_id = 7;
        super::super::run(&mut function);
        let result = function.blocks[0]
            .instructions
            .iter()
            .find(|inst| inst.dest() == Some(Value(6)))
            .unwrap();
        assert_eq!(
            matches!(
                result,
                Instruction::BinOp {
                    op: IrBinOp::Xor,
                    ..
                }
            ),
            !bare_use
        );
        assert_eq!(
            function.blocks[0].source_spans.len(),
            function.blocks[0].instructions.len()
        );
    }
}

#[test]
fn owner_skips_dead_networks_but_keeps_bare_effect_roots_live() {
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    for effect_root in [false, true] {
        let mut instructions = hidden_xor(IrType::U64);
        if effect_root {
            instructions.push(Instruction::StackRestore { ptr: Value(6) });
        }
        let mut function = IrFunction::new("live_logic".into(), IrType::U64, vec![], false);
        function.next_value_id = 7;
        function.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            source_spans: vec![],
            terminator: Terminator::Return(None),
        });
        assert_eq!(super::super::run(&mut function), usize::from(effect_root));
        let root = function.blocks[0]
            .instructions
            .iter()
            .find(|i| i.dest() == Some(Value(6)))
            .unwrap();
        assert_eq!(
            matches!(
                root,
                Instruction::BinOp {
                    op: IrBinOp::Xor,
                    ..
                }
            ),
            effect_root
        );
    }
}

#[test]
fn owner_reserves_all_temporaries_for_multiple_three_operation_recipes() {
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    let ty = IrType::U64;
    // Pick a minimal three-operation recipe outside specialized CH/MAJ
    // shapes, then make each copy intentionally redundant with double NOT.
    let bits = recipes()
        .iter()
        .enumerate()
        .find_map(|(bits, recipe)| {
            if recipe.as_ref()?.cost != 3 {
                return None;
            }
            let mut probe = Vec::new();
            let base = emit(bits as u8, &[v(0), v(1), v(2)], ty, Value(3), &mut probe)?;
            probe.push(Instruction::UnaryOp {
                dest: Value(6),
                op: IrUnaryOp::Not,
                src: base,
                ty,
            });
            probe.push(Instruction::UnaryOp {
                dest: Value(7),
                op: IrUnaryOp::Not,
                src: v(6),
                ty,
            });
            (rewrite(&probe, None)?.len() == 3).then_some(bits as u8)
        })
        .expect("an admitted three-operation recipe");
    let mut source = Vec::new();
    for first in [3, 8] {
        // emit uses the output length for fresh IDs, so each independent
        // recipe is emitted in its own temporary vector before appending.
        let mut network = Vec::new();
        let base = emit(bits, &[v(0), v(1), v(2)], ty, Value(first), &mut network).unwrap();
        network.push(Instruction::UnaryOp {
            dest: Value(first + 3),
            op: IrUnaryOp::Not,
            src: base,
            ty,
        });
        network.push(Instruction::UnaryOp {
            dest: Value(first + 4),
            op: IrUnaryOp::Not,
            src: v(first + 3),
            ty,
        });
        source.extend(network);
    }
    source.push(bin(Value(13), IrBinOp::Add, v(7), v(12), ty));
    let mut function = IrFunction::new("parallel_recipes".into(), ty, vec![], false);
    function.next_value_id = 14;
    function.blocks.push(BasicBlock {
        label: BlockId(0),
        instructions: source.clone(),
        source_spans: vec![Span::dummy(); source.len()],
        terminator: Terminator::Return(Some(v(13))),
    });
    assert!(super::super::run(&mut function) >= 2);
    assert!(function.next_value_id >= 18);
    let block = &function.blocks[0];
    let mut defined = std::collections::HashSet::new();
    for instruction in &block.instructions {
        if let Some(dest) = instruction.dest() {
            assert!(defined.insert(dest), "duplicate SSA destination: {dest:?}");
            assert!(dest.0 < function.next_value_id);
        }
    }
    assert_eq!(block.source_spans.len(), block.instructions.len());
    for inputs in [
        [0, 0, 0],
        [u64::MAX, 1, 0],
        [0x123456789abcdef0, 0xfedcba9876543210, 0xaaaa5555ffff0000],
    ] {
        assert_eq!(
            evaluate(&block.instructions, v(13), inputs, 64),
            evaluate(&source, v(13), inputs, 64)
        );
    }
}

#[test]
fn specialized_choose_and_majority_prefixes_remain_opaque() {
    let ty = IrType::U32;
    let choose = vec![
        bin(Value(3), IrBinOp::And, v(0), v(1), ty),
        Instruction::UnaryOp {
            dest: Value(4),
            op: IrUnaryOp::Not,
            src: v(0),
            ty,
        },
        bin(Value(5), IrBinOp::And, v(4), v(2), ty),
        bin(Value(6), IrBinOp::Xor, v(3), v(5), ty),
    ];
    assert!(rewrite(&choose, None).is_none());
    let prefix = vec![
        bin(Value(3), IrBinOp::And, v(0), v(1), ty),
        bin(Value(4), IrBinOp::And, v(0), v(2), ty),
        bin(Value(5), IrBinOp::Xor, v(3), v(4), ty),
    ];
    assert!(rewrite(&prefix, None).is_none());
}

#[test]
fn existing_boolean_algebra_kill_switch_disables_synthesis() {
    let source = hidden_xor(IrType::U32);
    assert!(rewrite_with_policy(&source, None, false).is_some());
    assert!(rewrite_with_policy(&source, None, true).is_none());
}
