//! Bit-exact lane-wise I32 DAG packing for the ordered reciprocal prefix.
//! The caller proves that external leaves dominate the preheader. Only integer
//! broadcasts/constants move there; all FP evaluation stays source-ordered.
//! Plans own speculative IDs/instructions, so every refusal is transactional.
use super::*;

pub(super) struct Plan {
    pub(super) denominator: Value,
    pub(super) prefix: Vec<Instruction>,
    pub(super) instructions: Vec<Instruction>,
    pub(super) next: u32,
}

struct Builder<'a> {
    body: &'a BasicBlock,
    cache: FxHashMap<Value, Value>,
    constants: FxHashMap<i32, Value>,
    expressions: FxHashMap<(IntrinsicOp, Value, u64), Value>,
    prefix: Vec<Instruction>,
    instructions: Vec<Instruction>,
    next: u32,
    operations: usize,
}
impl Builder<'_> {
    fn emit(&mut self, op: IntrinsicOp, args: Vec<Operand>, invariant: bool) -> Value {
        let dest = fresh_strict_value(&mut self.next);
        let inst = Instruction::Intrinsic {
            dest: Some(dest),
            op,
            args,
            dest_ptr: None,
        };
        if invariant {
            self.prefix.push(inst);
        } else {
            self.instructions.push(inst);
        }
        dest
    }
    fn operand(&mut self, operand: Operand, depth: usize) -> Option<Value> {
        if depth > 128 {
            return None;
        }
        let value = match operand {
            Operand::Const(c) => {
                // An unexpected wide constant must not be silently truncated.
                let n = i32::try_from(c.to_i64()?).ok()?;
                if let Some(&v) = self.constants.get(&n) {
                    return Some(v);
                }
                let v = self.emit(
                    IntrinsicOp::VecBroadcastI32x4,
                    vec![Operand::Const(IrConst::I32(n))],
                    true,
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
            None => self.emit(IntrinsicOp::VecBroadcastI32x4, vec![operand], true),
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
                // Only modular integer commutative operations may reorder.
                // This is not FP CSE or permission to move FP operations.
                let (a, b) = match b {
                    Operand::Value(v)
                        if matches!(
                            op,
                            IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
                        ) && v.0 < a.0 =>
                    {
                        (v, Operand::Value(a))
                    }
                    _ => (a, b),
                };
                let rhs_key = match b {
                    Operand::Value(v) => u64::from(v.0),
                    Operand::Const(c) => (1u64 << 32) | c.to_i64()? as u64,
                };
                let key = (packed_op, a, rhs_key);
                if let Some(&v) = self.expressions.get(&key) {
                    self.cache.insert(value, v);
                    return Some(v);
                }
                self.operations += 1;
                let v = self.emit(packed_op, vec![Operand::Value(a), b], false);
                self.expressions.insert(key, v);
                v
            }
            _ => return None,
        };
        self.cache.insert(value, result);
        Some(result)
    }
}

pub(super) fn build(
    body: &BasicBlock,
    iv: Value,
    lane_zero: Value,
    denominator: Value,
    next_id: u32,
) -> Option<Plan> {
    let mut b = Builder {
        body,
        cache: FxHashMap::default(),
        constants: FxHashMap::default(),
        expressions: FxHashMap::default(),
        prefix: Vec::new(),
        instructions: Vec::new(),
        next: next_id,
        operations: 0,
    };
    let zero = b.emit(
        IntrinsicOp::VecBroadcastI32x4,
        vec![Operand::Value(lane_zero)],
        false,
    );
    let offsets = b.emit(
        IntrinsicOp::VecPackI32x4,
        (0..4).map(|i| Operand::Const(IrConst::I32(i))).collect(),
        true,
    );
    let packed_iv = b.emit(
        IntrinsicOp::VecAddI32x4,
        vec![Operand::Value(zero), Operand::Value(offsets)],
        false,
    );
    b.cache.insert(iv, packed_iv);
    let denominator = b.operand(Operand::Value(denominator), 0)?;
    // Retain the synthesis's measured tiny-DAG cost fence. CSE is not a
    // reason to enlarge an otherwise cheap scalar coefficient expression.
    (b.operations >= 4).then_some(Plan {
        denominator,
        prefix: b.prefix,
        instructions: b.instructions,
        next: b.next,
    })
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
                let plan = build(&block, Value(0), Value(50), Value(5), 100).unwrap();
                let result = plan.denominator;
                for start in [i32::MIN, -50000, -1, 0, 1, 50000, i32::MAX - 3] {
                    let mut values = FxHashMap::<Value, [i32; 4]>::default();
                    values.insert(Value(50), [start; 4]);
                    for inst in plan.prefix.iter().chain(&plan.instructions) {
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
    fn rejected_plan_discards_partially_allocated_instructions_and_ids() {
        let mut block = body(IrBinOp::SDiv, Operand::Const(IrConst::I32(3)), 1);
        block.instructions.push(Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            ty: IrType::I32,
            lhs: Operand::Value(Value(55)),
            rhs: Operand::Value(Value(1)),
        });
        let before = format!("{block:?}");
        let next = 100;
        // IV packing and the external left broadcast precede the unsupported
        // right-hand division. Neither private allocation may escape failure.
        assert!(build(&block, Value(0), Value(50), Value(2), next).is_none());
        assert_eq!(next, 100);
        assert_eq!(format!("{block:?}"), before);
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
    fn builder(block: &BasicBlock) -> Builder<'_> {
        Builder {
            body: block,
            cache: FxHashMap::default(),
            constants: FxHashMap::default(),
            expressions: FxHashMap::default(),
            prefix: vec![],
            instructions: vec![],
            next: 100,
            operations: 0,
        }
    }

    #[test]
    fn packed_cse_preserves_noncommutative_operand_order() {
        for op in [
            IrBinOp::Add,
            IrBinOp::Mul,
            IrBinOp::And,
            IrBinOp::Or,
            IrBinOp::Xor,
            IrBinOp::Sub,
        ] {
            let mut block = body(op, Operand::Value(Value(80)), 1);
            block.instructions.push(Instruction::BinOp {
                dest: Value(2),
                op,
                ty: IrType::I32,
                lhs: Operand::Value(Value(80)),
                rhs: Operand::Value(Value(0)),
            });
            let mut b = builder(&block);
            let first = b.operand(Operand::Value(Value(1)), 0).unwrap();
            let second = b.operand(Operand::Value(Value(2)), 0).unwrap();
            assert_eq!(first == second, op != IrBinOp::Sub, "{op:?}");
            assert_eq!(
                b.prefix.len(),
                2,
                "each external integer leaf broadcasts once"
            );
        }
    }

    #[test]
    fn checked_constants_depth_and_complete_plan_ids() {
        let block = body(IrBinOp::Mul, Operand::Const(IrConst::I32(7)), 5);
        let mut b = builder(&block);
        for c in [i32::MIN, -1, 0, i32::MAX] {
            let operand = Operand::Const(IrConst::I32(c));
            let first = b.operand(operand, 0).unwrap();
            assert_eq!(b.operand(operand, 0), Some(first));
        }
        assert_eq!(b.prefix.len(), 4);
        assert!(
            b.operand(Operand::Const(IrConst::I64(1i64 << 32)), 0)
                .is_none()
        );
        assert!(b.operand(Operand::Value(Value(0)), 129).is_none());
        let plan = build(&block, Value(0), Value(50), Value(5), 100).unwrap();
        let ids: Vec<_> = plan
            .prefix
            .iter()
            .chain(&plan.instructions)
            .filter_map(Instruction::dest)
            .collect();
        assert_eq!(ids.len(), ids.iter().collect::<FxHashSet<_>>().len());
        assert!(ids.iter().all(|v| v.0 < plan.next));
        assert!(ids.contains(&plan.denominator));
        assert!(plan.prefix.iter().all(|i| matches!(
            i,
            Instruction::Intrinsic {
                op: IntrinsicOp::VecBroadcastI32x4 | IntrinsicOp::VecPackI32x4,
                ..
            }
        )));
    }
}
