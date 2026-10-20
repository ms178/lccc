use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

fn bin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty,
    }
}
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn c(n: i64, ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(n, ty))
}
fn function(ty: IrType, mut code: Vec<Instruction>) -> IrFunction {
    code.insert(
        0,
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        },
    );
    let mut f = IrFunction::new("exact".into(), ty, vec![], false);
    f.blocks.push(BasicBlock {
        label: BlockId(0),
        instructions: code,
        terminator: Terminator::Return(Some(v(3))),
        source_spans: vec![],
    });
    f.next_value_id = f.max_value_id() + 1;
    f
}

#[test]
fn opposite_parity_division_and_remainder_retain_destinations() {
    for ty in [IrType::I32, IrType::I64] {
        for op in [IrBinOp::Add, IrBinOp::Sub, IrBinOp::Xor] {
            for odd in [-7, -1, 1, 5] {
                for rem in [false, true] {
                    let mut f = function(
                        ty,
                        vec![
                            bin(1, op, v(0), c(odd, ty), ty),
                            bin(2, IrBinOp::Mul, v(0), v(1), ty),
                            bin(
                                3,
                                if rem { IrBinOp::SRem } else { IrBinOp::SDiv },
                                v(2),
                                c(2, ty),
                                ty,
                            ),
                        ],
                    );
                    assert_eq!(run(&mut f), 1);
                    assert_eq!(f.blocks[0].instructions[3].dest(), Some(Value(3)));
                    if rem {
                        assert!(matches!(
                            f.blocks[0].instructions[3],
                            Instruction::Copy {
                                src: Operand::Const(_),
                                ..
                            }
                        ));
                    } else {
                        assert!(matches!(
                            f.blocks[0].instructions[3],
                            Instruction::BinOp {
                                op: IrBinOp::AShr,
                                ..
                            }
                        ));
                    }
                    assert_eq!(run(&mut f), 0, "rewrite must be idempotent");
                }
            }
        }
    }
}

#[test]
fn insufficient_low_bits_even_offsets_and_negative_divisors_decline() {
    for ty in [IrType::I32, IrType::I64] {
        for (offset, divisor) in [(2, 2), (0, 2), (1, 4), (1, -2), (1, 0), (1, 3)] {
            let mut f = function(
                ty,
                vec![
                    bin(1, IrBinOp::Add, v(0), c(offset, ty), ty),
                    bin(2, IrBinOp::Mul, v(0), v(1), ty),
                    bin(3, IrBinOp::SDiv, v(2), c(divisor, ty), ty),
                ],
            );
            let before = format!("{f:?}");
            assert_eq!(run(&mut f), 0);
            assert_eq!(format!("{f:?}"), before);
        }
    }
}

#[test]
fn low_zero_facts_hold_for_modular_words_and_signed_boundaries() {
    for ty in [IrType::I32, IrType::I64] {
        let bits = width(ty).unwrap();
        let mask = u64::MAX >> (64 - bits);
        for op in [
            IrBinOp::And,
            IrBinOp::Or,
            IrBinOp::Xor,
            IrBinOp::Add,
            IrBinOp::Sub,
            IrBinOp::Mul,
            IrBinOp::Shl,
            IrBinOp::LShr,
            IrBinOp::AShr,
        ] {
            let rhs = if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                3
            } else {
                24
            };
            let f = function(
                ty,
                vec![
                    bin(1, IrBinOp::Shl, v(0), c(4, ty), ty),
                    bin(2, op, v(1), c(rhs, ty), ty),
                ],
            );
            let mut facts = Facts {
                defs: vec![None; f.max_value_id() as usize + 1],
                bits,
                budget: 64,
            };
            for i in &f.blocks[0].instructions {
                facts.defs[i.dest().unwrap().0 as usize] = Some(i);
            }
            let guaranteed = facts.zeros(v(2));
            for x in (0..65536u64).chain([u64::MAX, 1 << 31, (1 << 31) - 1, 1 << 63, (1 << 63) - 1])
            {
                let a = x.wrapping_shl(4) & mask;
                let result = match op {
                    IrBinOp::And => a & rhs as u64,
                    IrBinOp::Or => a | rhs as u64,
                    IrBinOp::Xor => a ^ rhs as u64,
                    IrBinOp::Add => a.wrapping_add(rhs as u64),
                    IrBinOp::Sub => a.wrapping_sub(rhs as u64),
                    IrBinOp::Mul => a.wrapping_mul(rhs as u64),
                    IrBinOp::Shl => a << rhs,
                    IrBinOp::LShr => a >> rhs,
                    IrBinOp::AShr => {
                        if bits == 32 {
                            ((a as i32) >> rhs) as u64
                        } else {
                            ((a as i64) >> rhs) as u64
                        }
                    }
                    _ => unreachable!(),
                } & mask;
                assert!(
                    result.trailing_zeros().min(bits) >= guaranteed,
                    "{ty:?} {op:?} {x:x}"
                );
            }
        }
    }
}

#[test]
fn duplicate_ssa_and_narrowing_views_fail_closed() {
    let mut duplicate = function(
        IrType::I32,
        vec![
            bin(1, IrBinOp::Shl, v(0), c(4, IrType::I32), IrType::I32),
            bin(1, IrBinOp::Mul, v(0), v(0), IrType::I32),
            bin(3, IrBinOp::SDiv, v(1), c(8, IrType::I32), IrType::I32),
        ],
    );
    assert_eq!(run(&mut duplicate), 0);
    let mut narrow = function(
        IrType::I64,
        vec![
            bin(1, IrBinOp::Shl, v(0), c(4, IrType::I64), IrType::I64),
            Instruction::Cast {
                dest: Value(2),
                src: v(1),
                from_ty: IrType::I64,
                to_ty: IrType::I8,
            },
            bin(3, IrBinOp::SDiv, v(2), c(8, IrType::I64), IrType::I64),
        ],
    );
    assert_eq!(run(&mut narrow), 0);
}

#[test]
fn duplicate_inlined_coefficients_compare_structurally_not_by_load_address() {
    for ty in [IrType::I32, IrType::I64] {
        let mut f = function(
            ty,
            vec![
                bin(1, IrBinOp::Add, v(0), c(9, ty), ty),
                bin(2, IrBinOp::Add, c(9, ty), v(0), ty),
                bin(4, IrBinOp::Add, v(2), c(1, ty), ty),
                bin(5, IrBinOp::Mul, v(1), v(4), ty),
                bin(3, IrBinOp::SDiv, v(5), c(2, ty), ty),
            ],
        );
        assert_eq!(run(&mut f), 1);
        for (inst, id) in [(1, 1), (2, 2)] {
            f.blocks[0].instructions[inst] = Instruction::Load {
                dest: Value(id),
                ptr: Value(0),
                ty,
                seg_override: crate::common::types::AddressSpace::Default,
                volatile: true,
            };
        }
        // Reset the consumer: two reads of the same volatile address need
        // NOT return the same bits, so the opposite-parity proof must fail.
        *f.blocks[0].instructions.last_mut().unwrap() = bin(3, IrBinOp::SDiv, v(5), c(2, ty), ty);
        assert_eq!(run(&mut f), 0);
    }
}
