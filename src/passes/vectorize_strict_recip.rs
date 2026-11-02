//! Owned, fallible planning of the strict reciprocal packed denominator DAG.
//! The original function is borrowed immutably. Only a completed plan exposes
//! its instructions and next-ID checkpoint to the parent transformation.
use super::*;

/// Transactional AVX2 lowering of the already-validated I32 coefficient DAG.
/// Only lane-wise modular arithmetic and constant in-range shifts are admitted.
/// Exactly four I32 lanes feed four F64 products. No unobserved upper lanes
/// or 256-bit denominator homes are necessary.
struct StrictPackedDag<'a> {
    func: &'a IrFunction,
    body: usize,
    iv: Value,
    packed_iv: Value,
    values: FxHashMap<Value, Value>,
    constants: FxHashMap<i32, Value>,
    expressions: FxHashMap<(IntrinsicOp, Value, u64), Value>,
    prefix: Vec<Instruction>,
    instructions: Vec<Instruction>,
    next: u32,
}

/// Only a completed plan can transfer IDs and instructions to the function.
pub(super) struct StrictPackedPlan {
    pub(super) denominator: Value,
    pub(super) next: u32,
    pub(super) prefix: Vec<Instruction>,
    pub(super) instructions: Vec<Instruction>,
}

impl StrictPackedDag<'_> {
    fn finish(mut self, root: Operand) -> Option<StrictPackedPlan> {
        let denominator = self.operand(root, 0)?;
        Some(StrictPackedPlan {
            denominator,
            next: self.next,
            prefix: self.prefix,
            instructions: self.instructions,
        })
    }

    fn emit(&mut self, op: IntrinsicOp, args: Vec<Operand>, invariant: bool) -> Value {
        let dest = fresh_strict_value(&mut self.next);
        let inst = Instruction::Intrinsic {
            dest: Some(dest),
            op,
            dest_ptr: None,
            args,
        };
        if invariant {
            self.prefix.push(inst);
        } else {
            self.instructions.push(inst);
        }
        dest
    }

    fn operand(&mut self, operand: Operand, depth: usize) -> Option<Value> {
        if depth > 64 {
            return None;
        }
        let value = match operand {
            Operand::Const(c) => {
                let c = i32::try_from(c.to_i64()?).ok()?;
                if let Some(&value) = self.constants.get(&c) {
                    return Some(value);
                }
                let value = self.emit(
                    IntrinsicOp::VecBroadcastI32x4,
                    vec![Operand::Const(IrConst::I32(c))],
                    true,
                );
                self.constants.insert(c, value);
                return Some(value);
            }
            Operand::Value(value) => value,
        };
        if value == self.iv {
            return Some(self.packed_iv);
        }
        if let Some(&packed) = self.values.get(&value) {
            return Some(packed);
        }
        let inst = find_inst_by_dest(&self.func.blocks[self.body], value).cloned();
        let result = match inst {
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
                let lhs = self.operand(lhs, depth + 1)?;
                let rhs = if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                    let Operand::Const(count) = rhs else {
                        return None;
                    };
                    let count = count.to_i64()?;
                    if !(0..32).contains(&count) {
                        return None;
                    }
                    Operand::Const(IrConst::I32(count as i32))
                } else {
                    Operand::Value(self.operand(rhs, depth + 1)?)
                };
                // Scalar copies may present the same commutative DAG in
                // different operand orders. Intern its packed operation once,
                // reducing transient register pressure without FP algebra.
                let (lhs, rhs) = match rhs {
                    Operand::Value(r)
                        if matches!(
                            op,
                            IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
                        ) && r.0 < lhs.0 =>
                    {
                        (r, Operand::Value(lhs))
                    }
                    _ => (lhs, rhs),
                };
                let rhs_key = match rhs {
                    Operand::Value(v) => u64::from(v.0),
                    Operand::Const(c) => (1u64 << 32) | c.to_i64()? as u64,
                };
                let key = (packed_op, lhs, rhs_key);
                if let Some(&value) = self.expressions.get(&key) {
                    return Some(value);
                }
                let value = self.emit(packed_op, vec![Operand::Value(lhs), rhs], false);
                self.expressions.insert(key, value);
                value
            }
            _ => return None,
        };
        self.values.insert(value, result);
        Some(result)
    }
}

/// Carried-IV variant of [`StrictPackedPlan`]: `packed_iv` is a loop-carried
/// XMM phi seeded to `[0, 1, 2, 3]` in `prefix` and advanced by `packed_iv_next`
/// (`vpaddd [4, 4, 4, 4]`) on the backedge, eliminating the per-iteration GPR
/// `vmovd` + `vpshufd` splat.
pub(super) struct StrictCarriedPackedPlan {
    pub(super) denominator: Value,
    pub(super) packed_iv_init: Value,
    pub(super) packed_iv_next: Value,
    pub(super) next: u32,
    pub(super) prefix: Vec<Instruction>,
    pub(super) instructions: Vec<Instruction>,
}

pub(super) fn build_plan(
    func: &IrFunction,
    pattern: &StrictComputedRecipReductionPattern,
    vector_iv: Value,
    next_value_id: u32,
) -> Option<StrictPackedPlan> {
    let mut speculative_next = next_value_id;
    let packed_iv = fresh_strict_value(&mut speculative_next);
    let mut packed = StrictPackedDag {
        func,
        body: pattern.body_idx,
        iv: pattern.iv,
        packed_iv,
        values: FxHashMap::default(),
        constants: FxHashMap::default(),
        expressions: FxHashMap::default(),
        prefix: Vec::new(),
        instructions: Vec::new(),
        next: speculative_next,
    };
    let offsets = packed.emit(
        IntrinsicOp::VecPackI32x4,
        (0..4)
            .map(|lane| Operand::Const(IrConst::I32(lane)))
            .collect(),
        true,
    );
    let splat = packed.emit(
        IntrinsicOp::VecBroadcastI32x4,
        vec![Operand::Value(vector_iv)],
        false,
    );
    packed.instructions.push(Instruction::Intrinsic {
        dest: Some(packed_iv),
        op: IntrinsicOp::VecAddI32x4,
        dest_ptr: None,
        args: vec![Operand::Value(splat), Operand::Value(offsets)],
    });
    packed.finish(Operand::Value(pattern.denominator))
}

pub(super) fn build_carried_plan(
    func: &IrFunction,
    pattern: &StrictComputedRecipReductionPattern,
    packed_iv: Value,
    next_value_id: u32,
) -> Option<StrictCarriedPackedPlan> {
    let mut packed = StrictPackedDag {
        func,
        body: pattern.body_idx,
        iv: pattern.iv,
        packed_iv,
        values: FxHashMap::default(),
        constants: FxHashMap::default(),
        expressions: FxHashMap::default(),
        prefix: Vec::new(),
        instructions: Vec::new(),
        next: next_value_id,
    };
    let packed_iv_init = packed.emit(
        IntrinsicOp::VecPackI32x4,
        (0..4)
            .map(|lane| Operand::Const(IrConst::I32(lane)))
            .collect(),
        true,
    );
    let step4 = packed.operand(Operand::Const(IrConst::I32(4)), 0)?;
    let denominator = packed.operand(Operand::Value(pattern.denominator), 0)?;
    let packed_iv_next = packed.emit(
        IntrinsicOp::VecAddI32x4,
        vec![Operand::Value(packed_iv), Operand::Value(step4)],
        false,
    );
    Some(StrictCarriedPackedPlan {
        denominator,
        packed_iv_init,
        packed_iv_next,
        next: packed.next,
        prefix: packed.prefix,
        instructions: packed.instructions,
    })
}

#[cfg(test)]
mod strict_packed_dag_tests {
    use super::*;

    fn function(instructions: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("packed_dag".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        f
    }

    fn builder(func: &IrFunction) -> StrictPackedDag<'_> {
        StrictPackedDag {
            func,
            body: 0,
            iv: Value(0),
            packed_iv: Value(100),
            next: 101,
            values: FxHashMap::default(),
            constants: FxHashMap::default(),
            expressions: FxHashMap::default(),
            prefix: vec![],
            instructions: vec![],
        }
    }

    fn bin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty: IrType::I32,
        }
    }

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

    fn scalar_chain(op: IrBinOp, rhs: Operand, count: u32) -> BasicBlock {
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
                let block = scalar_chain(op, Operand::Const(IrConst::I32(rhs)), 5);
                let f = function(block.instructions);
                // Exercise the production entry point, including its packed
                // lane initialization. CFG/reduction fields are not consumed
                // by this pure DAG planner; native gates test the full loop.
                let pattern = StrictComputedRecipReductionPattern {
                    header_idx: 0,
                    body_idx: 0,
                    preheader_idx: 0,
                    iv: Value(0),
                    sum_phi: Value(6),
                    sum_init: Operand::Const(IrConst::F64(0.0)),
                    limit: Operand::Const(IrConst::I32(4)),
                    denominator: Value(5),
                    source_base: Value(7),
                };
                let plan = build_plan(&f, &pattern, Value(0), 100).unwrap();
                // Five operations meet both planners' admission rules. Their
                // profitability/refusal domains are deliberately not equated.
                let reference = super::super::strict_i32_pack::build(
                    &f.blocks[0],
                    Value(0),
                    Value(0),
                    Value(5),
                    100,
                )
                .unwrap();
                for start in [i32::MIN, -50000, -1, 0, 1, 50000, i32::MAX - 3] {
                    let mut outputs = Vec::new();
                    for (implementation, result, prefix, instructions) in [
                        (
                            "production",
                            plan.denominator,
                            plan.prefix.as_slice(),
                            plan.instructions.as_slice(),
                        ),
                        (
                            "upstream reference",
                            reference.denominator,
                            reference.prefix.as_slice(),
                            reference.instructions.as_slice(),
                        ),
                    ] {
                        let mut values = FxHashMap::<Value, [i32; 4]>::default();
                        values.insert(Value(0), [start; 4]);
                        for inst in prefix.iter().chain(instructions) {
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
                            assert_eq!(
                                op.vector_home_width(),
                                Some(16),
                                "only 128-bit denominator values"
                            );
                            let a = get(args[0]);
                            let lanes = match op {
                                IntrinsicOp::VecBroadcastI32x4 => [a[0]; 4],
                                IntrinsicOp::VecPackI32x4 => {
                                    std::array::from_fn(|i| get(args[i])[0])
                                }
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
                                "{implementation} {op:?} {start} {rhs} {lane}"
                            );
                        }
                        outputs.push(values[&result]);
                    }
                    assert_eq!(outputs[0], outputs[1], "{op:?} {start} {rhs}");
                }
            }
        }
    }

    #[test]
    fn failed_speculation_drops_ids_and_complete_plan_owns_unique_ids() {
        let f = function(vec![bin(
            2,
            IrBinOp::SDiv,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I32(3)),
        )]);
        let mut b = builder(&f);
        let checkpoint = b.next;
        b.operand(Operand::Const(IrConst::I32(7)), 0).unwrap();
        assert!(
            b.next > checkpoint,
            "valid speculation advances its private counter"
        );
        assert!(b.finish(Operand::Value(Value(2))).is_none());
        let f = function(vec![bin(
            2,
            IrBinOp::Add,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I32(3)),
        )]);
        let plan = builder(&f).finish(Operand::Value(Value(2))).unwrap();
        let ids: Vec<_> = plan
            .prefix
            .iter()
            .chain(&plan.instructions)
            .filter_map(Instruction::dest)
            .collect();
        assert_eq!(ids.len(), ids.iter().collect::<FxHashSet<_>>().len());
        assert!(ids.iter().all(|v| v.0 < plan.next));
        assert!(ids.contains(&plan.denominator));
    }

    #[test]
    fn copy_index_rejects_duplicate_definitions_cycles_and_overlong_chains() {
        let mut f = function(vec![
            Instruction::Copy {
                dest: Value(10),
                src: Operand::Value(Value(4)),
            },
            Instruction::Copy {
                dest: Value(10),
                src: Operand::Value(Value(5)),
            },
        ]);
        assert!(reduction_definitions(&f).is_none());
        f.blocks[0].instructions.pop();
        assert_eq!(
            reduction_copy_root(&reduction_definitions(&f).unwrap(), Value(10)),
            Some(Value(4))
        );
        f.blocks[0].instructions = (0..10)
            .map(|n| Instruction::Copy {
                dest: Value(n),
                src: Operand::Value(Value(n + 1)),
            })
            .collect();
        assert_eq!(
            reduction_copy_root(&reduction_definitions(&f).unwrap(), Value(0)),
            None
        );
        f.blocks[0].instructions = vec![Instruction::Copy {
            dest: Value(0),
            src: Operand::Value(Value(0)),
        }];
        assert_eq!(
            reduction_copy_root(&reduction_definitions(&f).unwrap(), Value(0)),
            None
        );
        // A definition not traversed by this query does not poison a parameter.
        assert_eq!(
            reduction_copy_root(&reduction_definitions(&f).unwrap(), Value(99)),
            Some(Value(99))
        );
    }

    #[test]
    fn packed_integer_cse_is_commutative_but_subtraction_is_not() {
        for op in [
            IrBinOp::Add,
            IrBinOp::Mul,
            IrBinOp::And,
            IrBinOp::Or,
            IrBinOp::Xor,
            IrBinOp::Sub,
        ] {
            let iv = Operand::Value(Value(0));
            let external = Operand::Value(Value(1));
            let f = function(vec![bin(2, op, iv, external), bin(3, op, external, iv)]);
            let mut b = builder(&f);
            let a = b.operand(Operand::Value(Value(2)), 0).unwrap();
            let c = b.operand(Operand::Value(Value(3)), 0).unwrap();
            assert_eq!(a == c, op != IrBinOp::Sub, "{op:?}");
            assert_eq!(b.prefix.len(), 1, "broadcast external once");
        }
    }

    #[test]
    fn packed_shift_boundaries_and_unsupported_ops_fail_closed() {
        for op in [IrBinOp::Shl, IrBinOp::LShr, IrBinOp::AShr] {
            for count in [-1, 0, 1, 31, 32, 63] {
                let f = function(vec![bin(
                    2,
                    op,
                    Operand::Value(Value(0)),
                    Operand::Const(IrConst::I32(count)),
                )]);
                assert_eq!(
                    builder(&f).operand(Operand::Value(Value(2)), 0).is_some(),
                    (0..32).contains(&count),
                    "{op:?} {count}"
                );
            }
            let f = function(vec![bin(
                2,
                op,
                Operand::Value(Value(0)),
                Operand::Value(Value(1)),
            )]);
            assert!(builder(&f).operand(Operand::Value(Value(2)), 0).is_none());
        }
        for op in [IrBinOp::SDiv, IrBinOp::SRem] {
            let f = function(vec![bin(
                2,
                op,
                Operand::Value(Value(0)),
                Operand::Const(IrConst::I32(3)),
            )]);
            assert!(builder(&f).operand(Operand::Value(Value(2)), 0).is_none());
        }
    }

    #[test]
    fn rejected_packed_plan_discards_partially_allocated_ids() {
        let f = function(vec![
            bin(
                2,
                IrBinOp::SDiv,
                Operand::Value(Value(0)),
                Operand::Const(IrConst::I32(3)),
            ),
            bin(
                3,
                IrBinOp::Add,
                Operand::Value(Value(1)),
                Operand::Value(Value(2)),
            ),
        ]);
        let before = format!("{f:?}");
        let mut plan = builder(&f);
        let next = plan.next;
        assert!(plan.operand(Operand::Value(Value(3)), 0).is_none());
        assert!(
            plan.next > next,
            "left broadcast allocated before right operand failed"
        );
        assert!(!plan.prefix.is_empty());
        drop(plan);
        assert_eq!(
            format!("{f:?}"),
            before,
            "speculation never mutates caller IR/counter"
        );
    }

    #[test]
    fn packed_constants_are_signed_i32_and_depth_is_bounded() {
        let f = function(vec![]);
        let mut b = builder(&f);
        for c in [i32::MIN, -1, 0, i32::MAX] {
            let op = Operand::Const(IrConst::I32(c));
            let first = b.operand(op, 0).unwrap();
            assert_eq!(b.operand(op, 0), Some(first));
        }
        assert_eq!(b.prefix.len(), 4);
        assert!(
            b.operand(Operand::Const(IrConst::I64(1i64 << 32)), 0)
                .is_none()
        );
        assert!(b.operand(Operand::Value(Value(0)), 65).is_none());
    }
}
