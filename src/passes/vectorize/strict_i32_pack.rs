//! Bit-exact lane-wise I32 DAG packing for the ordered reciprocal prefix.
//! All FP evaluation remains in the existing ordered intrinsic. Variable
//! shifts, divides, narrow/wide conversions and unknown nodes use the old
//! scalar clone path. Plans are built off-tree and discarded on refusal.
use super::*;

pub(super) fn build(
    body: &BasicBlock,
    iv: Value,
    lane_zero: Value,
    denominator: Value,
    next_id: u32,
) -> Option<(Value, Vec<Instruction>, u32)> {
    struct Builder<'a> {
        body: &'a BasicBlock,
        cache: FxHashMap<Value, Value>,
        constants: FxHashMap<i32, Value>,
        instructions: Vec<Instruction>,
        next: u32,
        operations: usize,
    }
    impl Builder<'_> {
        fn emit(&mut self, op: IntrinsicOp, args: Vec<Operand>) -> Value {
            let dest = fresh_strict_value(&mut self.next);
            self.instructions.push(Instruction::Intrinsic {
                dest: Some(dest),
                op,
                args,
                dest_ptr: None,
            });
            dest
        }
        fn operand(&mut self, operand: Operand, depth: usize) -> Option<Value> {
            if depth > 128 {
                return None;
            }
            let value = match operand {
                Operand::Const(c) => {
                    let n = c.to_i64()? as i32;
                    if let Some(&v) = self.constants.get(&n) {
                        return Some(v);
                    }
                    let v = self.emit(
                        IntrinsicOp::VecBroadcastI32x4,
                        vec![Operand::Const(IrConst::I32(n))],
                    );
                    self.constants.insert(n, v);
                    return Some(v);
                }
                Operand::Value(v) => v,
            };
            if let Some(&v) = self.cache.get(&value) {
                return Some(v);
            }
            let result = match find_inst_by_dest(self.body, value).cloned() {
                None => self.emit(IntrinsicOp::VecBroadcastI32x4, vec![operand]),
                Some(
                    Instruction::Copy { src, .. }
                    | Instruction::Cast {
                        src,
                        from_ty: IrType::I32,
                        to_ty: IrType::I32,
                        ..
                    },
                ) => self.operand(src, depth + 1)?,
                Some(Instruction::BinOp {
                    op,
                    lhs,
                    rhs,
                    ty: IrType::I32,
                    ..
                }) => {
                    let packed_op = match op {
                        IrBinOp::Add => IntrinsicOp::VecAddI32x4,
                        IrBinOp::Sub => IntrinsicOp::VecSubI32x4,
                        IrBinOp::Mul => IntrinsicOp::VecMulI32x4,
                        IrBinOp::And => IntrinsicOp::VecAndI32x4,
                        IrBinOp::Or => IntrinsicOp::VecOrI32x4,
                        IrBinOp::Xor => IntrinsicOp::VecXorI32x4,
                        IrBinOp::Shl => IntrinsicOp::VecShlI32x4,
                        IrBinOp::LShr => IntrinsicOp::VecLShrI32x4,
                        IrBinOp::AShr => IntrinsicOp::VecAShrI32x4,
                        _ => return None,
                    };
                    let a = self.operand(lhs, depth + 1)?;
                    let b = if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                        let Operand::Const(c) = rhs else {
                            return None;
                        };
                        let count = c.to_i64()?;
                        if !(0..32).contains(&count) {
                            return None;
                        }
                        Operand::Const(IrConst::I32(count as i32))
                    } else {
                        Operand::Value(self.operand(rhs, depth + 1)?)
                    };
                    self.operations += 1;
                    self.emit(packed_op, vec![Operand::Value(a), b])
                }
                _ => return None,
            };
            self.cache.insert(value, result);
            Some(result)
        }
    }
    let mut b = Builder {
        body,
        cache: FxHashMap::default(),
        constants: FxHashMap::default(),
        instructions: Vec::new(),
        next: next_id,
        operations: 0,
    };
    // Broadcast j once, then add constant {0,1,2,3}; unlike four scalar
    // lane computations + pack this is independent of four GPR live ranges.
    let zero = b.emit(
        IntrinsicOp::VecBroadcastI32x4,
        vec![Operand::Value(lane_zero)],
    );
    let offsets = b.emit(
        IntrinsicOp::VecPackI32x4,
        (0..4).map(|i| Operand::Const(IrConst::I32(i))).collect(),
    );
    let packed_iv = b.emit(
        IntrinsicOp::VecAddI32x4,
        vec![Operand::Value(zero), Operand::Value(offsets)],
    );
    b.cache.insert(iv, packed_iv);
    let den = b.operand(Operand::Value(denominator), 0)?;
    // Do not enlarge tiny DAGs with a vector-pack and extra broadcasts.
    (b.operations >= 4).then_some((den, b.instructions, b.next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(op: IrBinOp, a: i32, b: i32) -> i32 {
        match op {
            IrBinOp::Add => a.wrapping_add(b),
            IrBinOp::Sub => a.wrapping_sub(b),
            IrBinOp::Mul => a.wrapping_mul(b),
            IrBinOp::And => a & b,
            IrBinOp::Or => a | b,
            IrBinOp::Xor => a ^ b,
            IrBinOp::Shl => a.wrapping_shl(b as u32),
            IrBinOp::LShr => ((a as u32) >> b) as i32,
            IrBinOp::AShr => a >> b,
            _ => panic!("unsupported test op"),
        }
    }

    fn body(op: IrBinOp, rhs: Operand, count: u32) -> BasicBlock {
        BasicBlock {
            label: BlockId(0),
            instructions: (0..count)
                .map(|i| Instruction::BinOp {
                    dest: Value(i + 1),
                    op,
                    lhs: Operand::Value(Value(i)),
                    rhs,
                    ty: IrType::I32,
                })
                .collect(),
            terminator: Terminator::Return(None),
            source_spans: vec![],
        }
    }

    #[test]
    fn packed_integer_dag_matches_modular_scalar_lanes() {
        for op in [
            IrBinOp::Add,
            IrBinOp::Sub,
            IrBinOp::Mul,
            IrBinOp::And,
            IrBinOp::Or,
            IrBinOp::Xor,
            IrBinOp::Shl,
            IrBinOp::LShr,
            IrBinOp::AShr,
        ] {
            for rhs in [0, 1, 7, 31] {
                let block = body(op, Operand::Const(IrConst::I32(rhs)), 5);
                let (result, plan, _) = build(&block, Value(0), Value(50), Value(5), 100).unwrap();
                for start in [i32::MIN, -50000, -1, 0, 1, 50000, i32::MAX - 3] {
                    let mut values = FxHashMap::<Value, [i32; 4]>::default();
                    values.insert(Value(50), [start; 4]);
                    for inst in &plan {
                        let Instruction::Intrinsic {
                            dest: Some(dest),
                            op,
                            args,
                            ..
                        } = inst
                        else {
                            panic!("unexpected node")
                        };
                        let get = |operand: Operand| match operand {
                            Operand::Const(c) => [c.to_i64().unwrap() as i32; 4],
                            Operand::Value(v) => values[&v],
                        };
                        let a = get(args[0]);
                        let lanes = match op {
                            IntrinsicOp::VecBroadcastI32x4 => [a[0]; 4],
                            IntrinsicOp::VecPackI32x4 => std::array::from_fn(|i| get(args[i])[0]),
                            _ => {
                                let operation = match op {
                                    IntrinsicOp::VecAddI32x4 => IrBinOp::Add,
                                    IntrinsicOp::VecSubI32x4 => IrBinOp::Sub,
                                    IntrinsicOp::VecMulI32x4 => IrBinOp::Mul,
                                    IntrinsicOp::VecAndI32x4 => IrBinOp::And,
                                    IntrinsicOp::VecOrI32x4 => IrBinOp::Or,
                                    IntrinsicOp::VecXorI32x4 => IrBinOp::Xor,
                                    IntrinsicOp::VecShlI32x4 => IrBinOp::Shl,
                                    IntrinsicOp::VecLShrI32x4 => IrBinOp::LShr,
                                    IntrinsicOp::VecAShrI32x4 => IrBinOp::AShr,
                                    _ => panic!("unexpected intrinsic"),
                                };
                                let b = get(args[1]);
                                std::array::from_fn(|i| scalar(operation, a[i], b[i]))
                            }
                        };
                        values.insert(*dest, lanes);
                    }
                    for lane in 0..4 {
                        let mut expected = start.wrapping_add(lane as i32);
                        for _ in 0..5 {
                            expected = scalar(op, expected, rhs);
                        }
                        assert_eq!(
                            values[&result][lane], expected,
                            "{op:?} {start} {rhs} {lane}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unsupported_shifts_division_and_small_dags_refuse_without_mutation() {
        for block in [
            body(IrBinOp::Shl, Operand::Const(IrConst::I32(-1)), 5),
            body(IrBinOp::AShr, Operand::Const(IrConst::I32(32)), 5),
            body(IrBinOp::LShr, Operand::Value(Value(55)), 5),
            body(IrBinOp::SDiv, Operand::Const(IrConst::I32(2)), 5),
            body(IrBinOp::Add, Operand::Const(IrConst::I32(1)), 3),
        ] {
            let before = format!("{block:?}");
            let den = Value(block.instructions.len() as u32);
            assert!(build(&block, Value(0), Value(50), den, 100).is_none());
            assert_eq!(before, format!("{block:?}"));
        }
    }
}
