//! Transactional adapter for finite, nonzero-based byte predicate counts.
//!
//! This does not weaken the packed transform's zero-IV/strict-bound contract.
//! It proves an equivalent scalar loop on a private function, then commits only
//! when the existing packed analyzer AND emitter accept that canonical loop.
use super::*;

fn constant(op: Operand) -> Option<i64> {
    if let Operand::Const(c) = op {
        c.to_i64()
    } else {
        None
    }
}
fn uses(inst: &Instruction, value: Value) -> bool {
    let mut found = false;
    inst.for_each_used_value(|v| found |= v == value.0);
    found
}
fn fresh(next: &mut u32) -> Value {
    let v = Value(*next);
    *next += 1;
    v
}

fn reserve_adapter_ids(func: &IrFunction) -> Option<u32> {
    // Reserve a small adapter's IDs without trusting stale metadata. The
    // ordinary packed emitter retains its own ID ownership discipline.
    let max_dest = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(Instruction::dest)
        .map(|v| v.0)
        .max()
        .unwrap_or(0);
    let next = func.next_value_id.max(max_dest.checked_add(1)?);
    next.checked_add(16384)?;
    Some(next)
}

struct CountReduction {
    select_site: (usize, usize),
    add_site: (usize, usize),
    selected: Value,
    acc: Value,
    acc_ty: IrType,
    cond: Value,
    cond_ty: IrType,
    predicate: IrCmpOp,
}

/// Identify the conditional scalar recurrence without mutating the function.
fn find_count_reduction(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    iv: Value,
    latch_label: BlockId,
) -> Option<CountReduction> {
    let header = lp.header;
    // Match acc' = cond ? acc + 1 : acc (or its reversed spelling).
    let mut reduction = None;
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi {
            dest: acc,
            ty: acc_ty,
            incoming,
        } = inst
        else {
            continue;
        };
        if *acc == iv
            || incoming.len() != 2
            || !matches!(
                acc_ty,
                IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64
            )
        {
            continue;
        }
        let Some((Operand::Value(selected), _)) = incoming.iter().find(|(_, l)| *l == latch_label)
        else {
            continue;
        };
        let Some(select_site) = definition_site(func, *selected) else {
            continue;
        };
        if !lp.contains(select_site.0) {
            continue;
        }
        let Instruction::Select {
            cond: Operand::Value(cond),
            true_val,
            false_val,
            ty: select_ty,
            ..
        } = func.blocks[select_site.0].instructions[select_site.1]
        else {
            continue;
        };
        if select_ty != *acc_ty {
            continue;
        }
        let (Operand::Value(added), predicate) = (if false_val == Operand::Value(*acc) {
            (true_val, IrCmpOp::Ne)
        } else if true_val == Operand::Value(*acc) {
            (false_val, IrCmpOp::Eq)
        } else {
            continue;
        }) else {
            continue;
        };
        let Some(add_site) = definition_site(func, added) else {
            continue;
        };
        if !lp.contains(add_site.0) {
            continue;
        }
        let Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty: add_ty,
            ..
        } = func.blocks[add_site.0].instructions[add_site.1]
        else {
            continue;
        };
        if add_ty != *acc_ty
            || !((lhs == Operand::Value(*acc) && constant(rhs) == Some(1))
                || (rhs == Operand::Value(*acc) && constant(lhs) == Some(1)))
        {
            continue;
        }
        let count = func
            .blocks
            .iter()
            .map(|b| {
                let mut n = 0;
                for i in &b.instructions {
                    i.for_each_used_value(|v| n += usize::from(v == added.0));
                }
                b.terminator
                    .for_each_used_value(|v| n += usize::from(v == added.0));
                n
            })
            .sum::<usize>();
        if count != 1 {
            continue;
        }
        let cond_site = definition_site(func, cond)?;
        let cond_ty = func.blocks[cond_site.0].instructions[cond_site.1].result_type()?;
        if !cond_ty.is_integer() {
            continue;
        }
        reduction = Some(CountReduction {
            select_site,
            add_site,
            selected: *selected,
            acc: *acc,
            acc_ty: *acc_ty,
            cond,
            cond_ty,
            predicate,
        });
        break;
    }
    reduction
}

/// Canonicalization is deliberately restricted to direct, nonnegative 32/64-bit
/// IVs (including exact 32-to-64-bit address casts) and constant bounds. Runtime starts, truncating index casts, zero-trip
/// cases, escaping IVs and overflowing inclusive endpoints fail closed.
fn normalize(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> Option<IrFunction> {
    let header = lp.header;
    let exit = find_exit(func, lp)?;
    let latch = find_latch(func, lp)?;
    if latch == header || cfg.preds.row(header).len() != 2 {
        return None;
    }
    let cmp_dest = byte_count_header_branch_cmp_dest(func, header, exit, &lp.body)?;
    let (cmp_block, cmp_index) = definition_site(func, cmp_dest)?;
    if cmp_block != header {
        return None;
    }
    let Instruction::Cmp {
        lhs: Operand::Value(iv),
        rhs,
        op,
        ty,
        ..
    } = func.blocks[header].instructions[cmp_index]
    else {
        return None;
    };
    let inclusive =
        match (ty, op) {
            (IrType::I32 | IrType::I64, IrCmpOp::Slt)
            | (IrType::U32 | IrType::U64, IrCmpOp::Ult) => false,
            (IrType::I32 | IrType::I64, IrCmpOp::Sle)
            | (IrType::U32 | IrType::U64, IrCmpOp::Ule) => true,
            _ => return None,
        };
    let end = constant(rhs)?.checked_add(i64::from(inclusive))?;
    let max = match ty {
        IrType::I32 => i64::from(i32::MAX),
        IrType::U32 => i64::from(u32::MAX),
        _ => i64::MAX,
    };
    if end > max {
        return None;
    }
    let (phi_block, phi_index) = definition_site(func, iv)?;
    if phi_block != header {
        return None;
    }
    let Instruction::Phi {
        incoming,
        ty: phi_ty,
        ..
    } = &func.blocks[header].instructions[phi_index]
    else {
        return None;
    };
    if *phi_ty != ty || incoming.len() != 2 {
        return None;
    }
    let latch_label = func.blocks[latch].label;
    let (initial, pre_label) = incoming.iter().find(|(_, l)| *l != latch_label)?;
    let start = constant(*initial)?;
    if start < 0 || end <= start {
        return None;
    }
    let pre = func.blocks.iter().position(|b| b.label == *pre_label)?;
    if lp.contains(pre)
        || !matches!(func.blocks[pre].terminator, Terminator::Branch(l) if l == func.blocks[header].label)
    {
        return None;
    }
    if !cfg.preds.row(header).contains(&(pre as u32))
        || !cfg.preds.row(header).contains(&(latch as u32))
    {
        return None;
    }
    let Operand::Value(step) = incoming.iter().find(|(_, l)| *l == latch_label)?.0 else {
        return None;
    };
    let step_site = definition_site(func, step)?;
    if !lp.contains(step_site.0) {
        return None;
    }
    let Instruction::BinOp {
        op: IrBinOp::Add,
        lhs,
        rhs,
        ty: step_ty,
        ..
    } = func.blocks[step_site.0].instructions[step_site.1]
    else {
        return None;
    };
    if step_ty != ty
        || !((lhs == Operand::Value(iv) && constant(rhs) == Some(1))
            || (rhs == Operand::Value(iv) && constant(lhs) == Some(1)))
    {
        return None;
    }
    if !lp.body.iter().any(|&b| {
        func.blocks[b].instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Load {
                    ty: IrType::I8 | IrType::U8,
                    volatile: false,
                    ..
                }
            )
        })
    }) {
        return None;
    }
    if !crate::passes::gvn::find_multi_def_values(func).is_empty() {
        return None;
    }

    // Only exact widening address casts are admitted. The finite nonnegative
    // trajectory makes sign- and zero-extension identical; a truncation or
    // arbitrary arithmetic index is not an alias of this rebased IV.
    let mut indices = FxHashSet::from_iter([iv]);
    let mut cast_sites = FxHashSet::default();
    for &bi in &lp.body {
        for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
            if let Instruction::Cast {
                dest,
                src: Operand::Value(src),
                from_ty,
                to_ty,
            } = inst
            {
                if *src == iv
                    && *from_ty == ty
                    && matches!(ty, IrType::I32 | IrType::U32)
                    && matches!(to_ty, IrType::I64 | IrType::U64)
                {
                    indices.insert(*dest);
                    cast_sites.insert((bi, ii));
                }
            }
        }
    }
    let mut geps = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if uses(inst, step) && (bi, ii) != (header, phi_index) {
                return None;
            }
            let mut reads_index = false;
            inst.for_each_used_value(|v| reads_index |= indices.contains(&Value(v)));
            if !reads_index {
                continue;
            }
            if (bi, ii) == (header, cmp_index)
                || (bi, ii) == step_site
                || cast_sites.contains(&(bi, ii))
            {
                continue;
            }
            let Instruction::GetElementPtr {
                base,
                offset: Operand::Value(index),
                ..
            } = inst
            else {
                return None;
            };
            if !indices.contains(index)
                || !lp.contains(bi)
                || !strict_external_value_available(func, cfg, &lp.body, pre, *base)
            {
                return None;
            }
            geps.push((bi, ii, *base));
        }
        let mut escapes = false;
        block
            .terminator
            .for_each_used_value(|v| escapes |= indices.contains(&Value(v)) || v == step.0);
        if escapes {
            return None;
        }
    }
    if geps.is_empty() || geps.len() > 8 {
        return None;
    }

    let CountReduction {
        select_site,
        add_site,
        selected,
        acc,
        acc_ty,
        cond,
        cond_ty,
        predicate,
    } = find_count_reduction(func, lp, iv, latch_label)?;
    let mut next = reserve_adapter_ids(func)?;
    let mut candidate = func.clone();
    if let Instruction::Phi { incoming, .. } = &mut candidate.blocks[header].instructions[phi_index]
    {
        for (value, label) in incoming {
            if *label == *pre_label {
                *value = Operand::Const(IrConst::I64(0));
            }
        }
    }
    if let Instruction::Cmp { rhs, op, .. } = &mut candidate.blocks[header].instructions[cmp_index]
    {
        *rhs = Operand::Const(IrConst::I64(end - start));
        *op = if matches!(ty, IrType::I32 | IrType::I64) {
            IrCmpOp::Slt
        } else {
            IrCmpOp::Ult
        };
    }
    for (bi, ii, base) in geps {
        if start == 0 {
            continue;
        }
        let shifted = fresh(&mut next);
        let block = &mut candidate.blocks[pre];
        if !block.source_spans.is_empty() {
            if block.source_spans.len() == block.instructions.len() {
                block
                    .source_spans
                    .push(crate::common::source::Span::dummy());
            } else {
                block.source_spans.clear();
            }
        }
        block.instructions.push(Instruction::GetElementPtr {
            dest: shifted,
            base,
            offset: Operand::Const(IrConst::I64(start)),
            ty: IrType::Ptr,
        });
        if let Instruction::GetElementPtr { base, .. } = &mut candidate.blocks[bi].instructions[ii]
        {
            *base = shifted;
        }
    }
    let truth = fresh(&mut next);
    let promoted = fresh(&mut next);
    for (bi, block) in candidate.blocks.iter_mut().enumerate() {
        if bi != select_site.0 && bi != add_site.0 {
            continue;
        }
        let old = std::mem::take(&mut block.instructions);
        let spans = std::mem::take(&mut block.source_spans);
        let mapped = spans.len() == old.len() && !spans.is_empty();
        for (ii, inst) in old.into_iter().enumerate() {
            if (bi, ii) == add_site {
                continue;
            }
            let before = block.instructions.len();
            if (bi, ii) == select_site {
                block.instructions.extend([
                    Instruction::Cmp {
                        dest: truth,
                        op: predicate,
                        lhs: Operand::Value(cond),
                        rhs: Operand::Const(IrConst::from_i64(0, cond_ty)),
                        ty: cond_ty,
                    },
                    Instruction::Cast {
                        dest: promoted,
                        src: Operand::Value(truth),
                        from_ty: IrType::I8,
                        to_ty: acc_ty,
                    },
                    Instruction::BinOp {
                        dest: selected,
                        op: IrBinOp::Add,
                        lhs: Operand::Value(acc),
                        rhs: Operand::Value(promoted),
                        ty: acc_ty,
                    },
                ]);
            } else {
                block.instructions.push(inst);
            }
            if mapped {
                block.source_spans.extend(std::iter::repeat_n(
                    spans[ii],
                    block.instructions.len() - before,
                ));
            }
        }
    }
    candidate.next_value_id = next;
    Some(candidate)
}

pub(super) fn transform(
    func: &mut IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> usize {
    let Some(mut candidate) = normalize(func, lp, cfg) else {
        return 0;
    };
    let Some(pattern) = analyze_byte_count_loop(&candidate, lp, cfg) else {
        return 0;
    };
    let changed = transform_byte_count_reduction(&mut candidate, &pattern);
    if changed != 0 {
        *func = candidate;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;

    fn fixture(start: i64, length: i64, inclusive: bool, reverse: bool, seed: i32) -> IrFunction {
        let c = |v| Operand::Const(IrConst::I64(v));
        let v = |id| Operand::Value(Value(id));
        let mut f = IrFunction::new("count".into(), IrType::I32, vec![], false);
        let block = |id, instructions: Vec<Instruction>, terminator| BasicBlock {
            label: BlockId(id),
            source_spans: vec![Span::dummy(); instructions.len()],
            instructions,
            terminator,
        };
        f.blocks = vec![
            block(
                0,
                vec![Instruction::GlobalAddr {
                    dest: Value(20),
                    name: "bytes".into(),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(0),
                        ty: IrType::I64,
                        incoming: vec![(c(start), BlockId(0)), (v(5), BlockId(2))],
                    },
                    Instruction::Phi {
                        dest: Value(1),
                        ty: IrType::I32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(seed)), BlockId(0)),
                            (v(4), BlockId(2)),
                        ],
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: if inclusive {
                            IrCmpOp::Sle
                        } else {
                            IrCmpOp::Slt
                        },
                        lhs: v(0),
                        rhs: c(start + length - i64::from(inclusive)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: v(2),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![
                    Instruction::GetElementPtr {
                        dest: Value(3),
                        base: Value(20),
                        offset: v(0),
                        ty: IrType::Ptr,
                    },
                    Instruction::Load {
                        dest: Value(6),
                        ptr: Value(3),
                        ty: IrType::I8,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::BinOp {
                        dest: Value(7),
                        op: IrBinOp::Add,
                        lhs: v(1),
                        rhs: c(1),
                        ty: IrType::I32,
                    },
                    Instruction::Select {
                        dest: Value(4),
                        cond: v(6),
                        true_val: if reverse { v(1) } else { v(7) },
                        false_val: if reverse { v(7) } else { v(1) },
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(5),
                        op: IrBinOp::Add,
                        lhs: v(0),
                        rhs: c(1),
                        ty: IrType::I64,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(v(1)))),
        ];
        f.pass_policy.wrap_signed_overflow = true;
        f.next_value_id = 21;
        f.next_label = 4;
        f
    }
    fn u32_fixture(start: u32, limit: u32, inclusive: bool) -> IrFunction {
        let length = i64::from(limit) - i64::from(start) + i64::from(inclusive);
        let mut f = fixture(i64::from(start), length, inclusive, false, 0);
        if let Instruction::Phi { ty, incoming, .. } = &mut f.blocks[1].instructions[0] {
            *ty = IrType::U32;
            incoming[0].0 = Operand::Const(IrConst::from_i64(i64::from(start), IrType::U32));
        }
        if let Instruction::Cmp { op, rhs, ty, .. } = &mut f.blocks[1].instructions[2] {
            *op = if inclusive {
                IrCmpOp::Ule
            } else {
                IrCmpOp::Ult
            };
            *rhs = Operand::Const(IrConst::from_i64(i64::from(limit), IrType::U32));
            *ty = IrType::U32;
        }
        if let Instruction::BinOp { rhs, ty, .. } = &mut f.blocks[2].instructions[4] {
            *rhs = Operand::Const(IrConst::from_i64(1, IrType::U32));
            *ty = IrType::U32;
        }
        f
    }

    fn lp() -> loop_analysis::NaturalLoop {
        loop_analysis::NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        }
    }
    fn read(op: Operand, values: &FxHashMap<Value, u64>) -> u64 {
        match op {
            Operand::Const(c) => c.to_i64().unwrap() as u64,
            Operand::Value(v) => values[&v],
        }
    }
    fn mask(value: u64, ty: IrType) -> u64 {
        if ty.size() == 8 {
            value
        } else {
            value & ((1u64 << (8 * ty.size())) - 1)
        }
    }
    // Execute the actual rewritten scalar CFG, rather than merely recomputing
    // the proposed mathematical bound or inspecting the rewritten opcode.
    fn interpret(f: &IrFunction, bytes: &[u8]) -> u32 {
        let mut values = FxHashMap::default();
        let mut current = BlockId(0);
        let mut previous = BlockId(u32::MAX);
        for _ in 0..4096 {
            let b = f.blocks.iter().find(|b| b.label == current).unwrap();
            let old = values.clone();
            for i in &b.instructions {
                if let Instruction::Phi { dest, incoming, ty } = i {
                    let op = incoming
                        .iter()
                        .find(|(_, label)| *label == previous)
                        .unwrap()
                        .0;
                    values.insert(*dest, mask(read(op, &old), *ty));
                }
            }
            for i in &b.instructions {
                let pair = match *i {
                    Instruction::Phi { .. } => continue,
                    Instruction::GlobalAddr { dest, .. } => (dest, 0),
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => (dest, values[&base].wrapping_add(read(offset, &values))),
                    Instruction::Load { dest, ptr, .. } => {
                        (dest, bytes[values[&ptr] as usize] as i8 as i64 as u64)
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty,
                    } => (
                        dest,
                        mask(read(lhs, &values).wrapping_add(read(rhs, &values)), ty),
                    ),
                    Instruction::Cmp {
                        dest, op, lhs, rhs, ..
                    } => (
                        dest,
                        u64::from(
                            op.eval_i64(read(lhs, &values) as i64, read(rhs, &values) as i64),
                        ),
                    ),
                    Instruction::Select {
                        dest,
                        cond,
                        true_val,
                        false_val,
                        ty,
                    } => (
                        dest,
                        mask(
                            read(
                                if read(cond, &values) != 0 {
                                    true_val
                                } else {
                                    false_val
                                },
                                &values,
                            ),
                            ty,
                        ),
                    ),
                    Instruction::Cast {
                        dest, src, to_ty, ..
                    } => (dest, mask(read(src, &values), to_ty)),
                    _ => panic!("unexpected instruction: {i:?}"),
                };
                values.insert(pair.0, pair.1);
            }
            previous = current;
            current = match b.terminator {
                Terminator::Branch(next) => next,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if read(cond, &values) != 0 {
                        true_label
                    } else {
                        false_label
                    }
                }
                Terminator::Return(Some(value)) => return read(value, &values) as u32,
                _ => panic!("unexpected terminator"),
            };
        }
        panic!("nonterminating count fixture")
    }

    #[test]
    fn interpreted_rebased_counts_preserve_offsets_truth_and_wrapping_accumulators() {
        for start in [0, 1, 2, 31] {
            for length in [1, 31, 32, 33, 64, 65] {
                for inclusive in [false, true] {
                    for reverse in [false, true] {
                        for seed in [-3, 7, i32::MAX - 3] {
                            let f = fixture(start, length, inclusive, reverse, seed);
                            let before = format!("{f:?}");
                            let normalized = normalize(&f, &lp(), &CfgAnalysis::build(&f)).unwrap();
                            assert_eq!(format!("{f:?}"), before);
                            assert!(
                                normalized
                                    .blocks
                                    .iter()
                                    .all(|b| b.source_spans.len() == b.instructions.len())
                            );
                            for data in [
                                vec![0; 128],
                                vec![255; 128],
                                (0..128).map(|i| [0, 1, 128, 255][i % 4]).collect(),
                            ] {
                                assert_eq!(interpret(&f, &data), interpret(&normalized, &data));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn interpreted_32bit_address_casts_keep_the_original_byte_stream() {
        for ty in [IrType::I32, IrType::U32] {
            for start in [0, 2, 31] {
                for length in [1, 31, 32, 33, 65] {
                    let mut f = fixture(start, length, true, false, -3);
                    if let Instruction::Phi { ty: t, .. } = &mut f.blocks[1].instructions[0] {
                        *t = ty;
                    }
                    if let Instruction::Cmp { ty: t, op, .. } = &mut f.blocks[1].instructions[2] {
                        *t = ty;
                        *op = if ty == IrType::I32 {
                            IrCmpOp::Sle
                        } else {
                            IrCmpOp::Ule
                        };
                    }
                    if let Instruction::BinOp { ty: t, .. } = &mut f.blocks[2].instructions[4] {
                        *t = ty;
                    }
                    if let Instruction::GetElementPtr { offset, .. } =
                        &mut f.blocks[2].instructions[0]
                    {
                        *offset = Operand::Value(Value(8));
                    }
                    f.blocks[2].instructions.insert(
                        0,
                        Instruction::Cast {
                            dest: Value(8),
                            src: Operand::Value(Value(0)),
                            from_ty: ty,
                            to_ty: IrType::I64,
                        },
                    );
                    f.blocks[2].source_spans.insert(0, Span::dummy());
                    let data: Vec<u8> = (0..128).map(|i| [0, 128, 255][i % 3]).collect();
                    let normalized = normalize(&f, &lp(), &CfgAnalysis::build(&f)).unwrap();
                    assert_eq!(interpret(&f, &data), interpret(&normalized, &data));
                    // The same index cannot also escape to an integer reader.
                    f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(8))));
                    assert!(normalize(&f, &lp(), &CfgAnalysis::build(&f)).is_none());
                }
            }
        }
    }

    #[test]
    fn u32_inclusive_endpoint_must_include_the_failing_header_domain() {
        // At U32::MAX the original inclusive loop wraps its IV after the last
        // body execution and never reaches a failing header test. Rebasing it
        // to a finite half-open range would change termination semantics.
        let mut overflowing = u32_fixture(0, u32::MAX, true);
        let before = format!("{overflowing:?}");
        let cfg = CfgAnalysis::build(&overflowing);
        assert!(normalize(&overflowing, &lp(), &cfg).is_none());
        assert_eq!(transform(&mut overflowing, &lp(), &cfg), 0);
        assert_eq!(format!("{overflowing:?}"), before);

        // The immediately preceding inclusive bound is legal: its failing
        // header observation is still representable in U32. Keep the rewritten
        // compare constant in the IR's U32 representation as well.
        let valid = u32_fixture(0, u32::MAX - 1, true);
        let normalized = normalize(&valid, &lp(), &CfgAnalysis::build(&valid)).unwrap();
        let Instruction::Cmp { rhs, ty, .. } = normalized.blocks[1].instructions[2] else {
            panic!("expected the header comparison")
        };
        assert_eq!(ty, IrType::U32);
        assert_eq!(
            rhs,
            Operand::Const(IrConst::from_i64(i64::from(u32::MAX), IrType::U32))
        );
    }

    #[test]
    fn rejected_adapter_and_failed_packed_plan_do_not_mutate_ir_or_metadata() {
        let base = fixture(2, 65, true, false, 7);
        let mut bad = Vec::new();
        bad.push(fixture(-1, 65, true, false, 7));
        bad.push(fixture(2, 0, true, false, 7));
        let mut overflow = base.clone();
        if let Instruction::Cmp { rhs, .. } = &mut overflow.blocks[1].instructions[2] {
            *rhs = Operand::Const(IrConst::I64(i64::MAX));
        }
        bad.push(overflow);
        let mut escape = base.clone();
        escape.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(0))));
        bad.push(escape);
        let mut step = base.clone();
        if let Instruction::BinOp { rhs, .. } = &mut step.blocks[2].instructions[4] {
            *rhs = Operand::Const(IrConst::I64(2));
        }
        bad.push(step);
        let mut dynamic = base.clone();
        if let Instruction::Phi { incoming, .. } = &mut dynamic.blocks[1].instructions[0] {
            incoming[0].0 = Operand::Value(Value(20));
        }
        bad.push(dynamic);
        let mut polarity = base.clone();
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &mut polarity.blocks[1].terminator
        {
            std::mem::swap(true_label, false_label);
        }
        bad.push(polarity);
        let mut duplicate = base.clone();
        duplicate.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I64(2)),
        });
        bad.push(duplicate);
        // Scalar normalization is possible, but the packed analyzer must reject
        // the observable write. The candidate and its fresh IDs are discarded.
        let mut write = base.clone();
        write.blocks[2].instructions.push(Instruction::Store {
            val: Operand::Value(Value(1)),
            ptr: Value(20),
            ty: IrType::I32,
            volatile: true,
            seg_override: AddressSpace::Default,
        });
        assert!(normalize(&write, &lp(), &CfgAnalysis::build(&write)).is_some());
        bad.push(write);
        for mut f in bad {
            let before = format!("{f:?}");
            let cfg = CfgAnalysis::build(&f);
            assert_eq!(transform(&mut f, &lp(), &cfg), 0);
            assert_eq!(format!("{f:?}"), before);
        }
    }
}
