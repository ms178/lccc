//! Load-only unsigned reductions: permit exact-trip partial unrolling, then
//! balance private additions so a group has only ONE loop-carried add.
//! No FP reassociation, speculative loads, extra iterations or wider accesses.
use super::*;
thread_local! { static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(crate) fn set_enabled(value: bool) {
    ENABLED.with(|v| v.set(value));
}

fn reads(i: &Instruction, mut visit: impl FnMut(&Value)) {
    crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
        if let Operand::Value(v) = o {
            visit(v);
        }
    });
    crate::backend::liveness::for_each_value_use_in_instruction(i, visit);
}
pub(super) fn candidate(f: &IrFunction, header: usize, latch: usize, iv: Value) -> Option<Value> {
    if !ENABLED.with(|v| v.get()) {
        return None;
    }
    let phis: Vec<_> = f.blocks[header]
        .instructions
        .iter()
        .filter_map(|i| {
            if let Instruction::Phi { dest, ty, incoming } = i {
                Some((*dest, *ty, incoming))
            } else {
                None
            }
        })
        .collect();
    if phis.len() != 2 {
        return None;
    }
    let (sum, ty, incoming) = phis.into_iter().find(|(v, _, _)| *v != iv)?;
    if !matches!(ty, IrType::U32 | IrType::U64) {
        return None;
    }
    let Operand::Value(back) = incoming
        .iter()
        .find(|(_, label)| *label == f.blocks[latch].label)?
        .0
    else {
        return None;
    };
    let mut sum_reads = 0;
    let mut back_uses = 0;
    for b in &f.blocks {
        for i in &b.instructions {
            reads(i, |v| {
                if *v == back {
                    back_uses += 1;
                }
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |o| {
            if *o == Operand::Value(back) {
                back_uses += 1;
            }
        });
    }
    if back_uses != 1 {
        return None;
    }
    let mut valid_update = false;
    for i in &f.blocks[latch].instructions {
        reads(i, |v| {
            if *v == sum {
                sum_reads += 1;
            }
        });
        match i {
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(a),
                ty: t,
                ..
            } if *dest == back && *a == sum && *t == ty => valid_update = true,
            Instruction::Load {
                volatile: false, ..
            } => {}
            Instruction::Store { .. } | Instruction::Load { volatile: true, .. } => return None,
            _ => {}
        }
    }
    (valid_update && sum_reads == 1).then_some(sum)
}

/// Construct the replacement before touching the cloned body. All consumed
/// partial sums must be private, and the original accumulator may occur only
/// at the head of the chain. The final SSA result and every input stay intact.
pub(super) fn balance(
    body: &mut Vec<Instruction>,
    accumulator: Value,
    root: Value,
    k: usize,
) -> bool {
    if !matches!(k, 2 | 4) {
        return false;
    }
    let mut dests = Vec::new();
    let mut inputs = Vec::new();
    let mut current = root;
    let mut ty = None;
    for _ in 0..k {
        let matches: Vec<_> = body.iter().filter(|i| i.dest() == Some(current)).collect();
        let [
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(previous),
                rhs,
                ty: t,
                ..
            },
        ] = matches.as_slice()
        else {
            return false;
        };
        if !matches!(t, IrType::U32 | IrType::U64) || ty.is_some_and(|ty| ty != *t) {
            return false;
        }
        ty = Some(*t);
        dests.push(current);
        inputs.push(*rhs);
        current = *previous;
    }
    if current != accumulator {
        return false;
    }
    for dest in dests.iter().skip(1) {
        let mut count = 0;
        for i in body.iter() {
            reads(i, |v| {
                if v == dest {
                    count += 1;
                }
            });
        }
        if count != 1 {
            return false;
        }
    }
    dests.reverse();
    inputs.reverse();
    let Some(t) = ty else {
        return false;
    };
    let add = |dest, lhs, rhs| Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs,
        rhs,
        ty: t,
    };
    let mut replacement = vec![add(dests[0], inputs[0], inputs[1])];
    if k == 4 {
        replacement.push(add(dests[1], inputs[2], inputs[3]));
        replacement.push(add(
            dests[2],
            Operand::Value(dests[0]),
            Operand::Value(dests[1]),
        ));
    }
    replacement.push(add(
        root,
        Operand::Value(accumulator),
        Operand::Value(dests[k - 2]),
    ));
    body.retain(|i| !i.dest().is_some_and(|v| dests.contains(&v)));
    body.extend(replacement);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chain(k: usize, ty: IrType) -> Vec<Instruction> {
        (0..k)
            .map(|j| Instruction::BinOp {
                dest: Value(10 + j as u32),
                op: IrBinOp::Add,
                lhs: Operand::Value(if j == 0 {
                    Value(0)
                } else {
                    Value(9 + j as u32)
                }),
                rhs: Operand::Value(Value(1 + j as u32)),
                ty,
            })
            .collect()
    }
    fn evaluate(body: &[Instruction], seed: u64, ty: IrType) -> u64 {
        let mut v = [0u64; 32];
        for (j, x) in v.iter_mut().take(5).enumerate() {
            *x = seed
                .rotate_left(j as u32 * 11)
                .wrapping_mul(0x9e3779b97f4a7c15);
        }
        for i in body {
            if let Instruction::BinOp {
                dest,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } = i
            {
                let n = v[a.0 as usize].wrapping_add(v[b.0 as usize]);
                v[dest.0 as usize] = if ty == IrType::U32 {
                    n as u32 as u64
                } else {
                    n
                };
            } else {
                panic!("unexpected instruction");
            }
        }
        v[body.last().unwrap().dest().unwrap().0 as usize]
    }
    #[test]
    fn balanced_reductions_preserve_wrapping_results() {
        for ty in [IrType::U32, IrType::U64] {
            for k in [2, 4] {
                let original = chain(k, ty);
                let mut b = original.clone();
                assert!(balance(&mut b, Value(0), Value(9 + k as u32), k));
                for seed in 0..4096 {
                    assert_eq!(evaluate(&original, seed, ty), evaluate(&b, seed, ty));
                }
                assert_eq!(b.len(), k);
            }
        }
    }
    #[test]
    fn declines_signed_and_unsupported_factors_transactionally() {
        for (k, ty) in [
            (2, IrType::I32),
            (4, IrType::I64),
            (8, IrType::U64),
            (1, IrType::U32),
        ] {
            let mut b = chain(k, ty);
            let before = format!("{b:?}");
            assert!(!balance(&mut b, Value(0), Value(9 + k as u32), k));
            assert_eq!(format!("{b:?}"), before);
        }
    }
    #[test]
    fn declines_shared_partial_sum_or_contribution_depending_on_it() {
        for bare in [false, true] {
            let mut b = chain(4, IrType::U32);
            b.push(if bare {
                Instruction::Load {
                    dest: Value(20),
                    ptr: Value(10),
                    seg_override: crate::common::types::AddressSpace::Default,
                    ty: IrType::U32,
                    volatile: false,
                }
            } else {
                Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Value(Value(10)),
                }
            });
            let before = format!("{b:?}");
            assert!(!balance(&mut b, Value(0), Value(13), 4));
            assert_eq!(format!("{b:?}"), before);
        }
        let mut b = chain(4, IrType::U32);
        if let Instruction::BinOp { rhs, .. } = &mut b[2] {
            *rhs = Operand::Value(Value(10));
        }
        assert!(!balance(&mut b, Value(0), Value(13), 4));
    }
    #[test]
    fn declines_duplicate_definitions_and_broken_chain() {
        let mut b = chain(4, IrType::U64);
        b.push(b[0].clone());
        assert!(!balance(&mut b, Value(0), Value(13), 4));
        let mut b = chain(4, IrType::U64);
        if let Instruction::BinOp { lhs, .. } = &mut b[0] {
            *lhs = Operand::Value(Value(2));
        }
        assert!(!balance(&mut b, Value(0), Value(13), 4));
    }
}
