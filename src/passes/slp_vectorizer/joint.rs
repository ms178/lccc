//! Joint narrow-store/reduction seeds. All widening arithmetic still uses
//! the ordinary SLP graph builder, scheduler, external-use and alias proofs.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Tail {
    pub alpha: usize,
    pub four_inputs: bool,
}

impl Tail {
    pub fn control(self) -> i32 {
        self.alpha as i32 | if self.four_inputs { 4 } else { 0 }
    }
    pub fn cost(self) -> i64 {
        match (self.four_inputs, x86_avx2_available_pub()) {
            (false, true) => 7,
            (true, true) => 9,
            (false, false) => 9,
            (true, false) => 12,
        }
    }
}

pub(super) fn barrier(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::Fence { .. }
            | Instruction::AtomicLoad { .. }
            | Instruction::Load { volatile: true, .. }
            | Instruction::Store { volatile: true, .. }
    )
}

pub(super) fn memory_access(inst: &Instruction) -> bool {
    barrier(inst) || is_memory_access(inst)
}

fn definition<'a>(ctx: &'a BlockCtx, o: &Operand) -> Option<&'a Instruction> {
    let Operand::Value(v) = o else { return None };
    Some(&ctx.block.instructions[*ctx.def_pos.get(&v.0)?])
}

/// Resolve only exact, default-segment nonvolatile byte readbacks. Unknown
/// writes, atomics, calls and asm stop the scan. Distinct offsets on the SAME
/// symbolic stream may be crossed; no guessed noalias relation is used.
fn forwarded(ctx: &BlockCtx, o: &Operand) -> Option<Operand> {
    let Operand::Value(v) = o else { return None };
    let &pos = ctx.def_pos.get(&v.0)?;
    let Instruction::Load {
        ptr,
        ty: IrType::U8,
        volatile: false,
        seg_override: AddressSpace::Default,
        ..
    } = &ctx.block.instructions[pos]
    else {
        return None;
    };
    let addr = eval_sym_addr(ctx.block, &ctx.def_pos, *ptr)?;
    for inst in ctx.block.instructions[..pos].iter().rev() {
        if barrier(inst) {
            return None;
        }
        if !is_memory_write(inst) {
            continue;
        }
        let Instruction::Store {
            ptr,
            val,
            ty,
            volatile: false,
            seg_override: AddressSpace::Default,
        } = inst
        else {
            return None;
        };
        let other = eval_sym_addr(ctx.block, &ctx.def_pos, *ptr)?;
        if (addr.base, addr.var, addr.mult) != (other.base, other.var, other.mult) {
            return None;
        }
        if other.off == addr.off && *ty == IrType::U8 {
            return Some(val.clone());
        }
        if byte_ranges_overlap(addr.off, 1, other.off, ty.size() as i64) {
            return None;
        }
    }
    None
}

fn canonical(ctx: &BlockCtx, o: &Operand, fuel: usize) -> Operand {
    if fuel == 0 {
        return o.clone();
    }
    let next = match definition(ctx, o) {
        Some(Instruction::Copy { src, .. }) => Some(src.clone()),
        Some(Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        }) if ty_is_integer(*from_ty)
            && ty_is_integer(*to_ty)
            && byte_bounded(ctx, src, fuel - 1) =>
        {
            Some(src.clone())
        }
        Some(Instruction::Load { .. }) => forwarded(ctx, o),
        _ => None,
    };
    next.map_or_else(|| o.clone(), |n| canonical(ctx, &n, fuel - 1))
}

/// Exact integer min/max spellings, including mirrored and non-strict
/// predicates. Ties are identical integer values; unlike FP, operand order
/// on a tie does not affect the result.
fn minmax(ctx: &BlockCtx, o: &Operand) -> Option<(bool, Operand, Operand, bool)> {
    let Instruction::Select {
        cond,
        true_val,
        false_val,
        ..
    } = definition(ctx, o)?
    else {
        return None;
    };
    let Instruction::Cmp {
        op, lhs, rhs, ty, ..
    } = definition(ctx, cond)?
    else {
        return None;
    };
    if !matches!(ty, IrType::I32 | IrType::U32) {
        return None;
    }
    let (greater, unsigned) = match op {
        IrCmpOp::Sgt | IrCmpOp::Sge => (true, false),
        IrCmpOp::Slt | IrCmpOp::Sle => (false, false),
        IrCmpOp::Ugt | IrCmpOp::Uge => (true, true),
        IrCmpOp::Ult | IrCmpOp::Ule => (false, true),
        _ => return None,
    };
    let same = |a: &Operand, b: &Operand| {
        same_source(ctx, a, b) || canonical(ctx, a, 8) == canonical(ctx, b, 8)
    };
    if same(true_val, lhs) && same(false_val, rhs) {
        Some((greater, lhs.clone(), rhs.clone(), unsigned))
    } else if same(true_val, rhs) && same(false_val, lhs) {
        Some((!greater, lhs.clone(), rhs.clone(), unsigned))
    } else {
        None
    }
}

fn bounds(ctx: &BlockCtx, o: &Operand, fuel: usize) -> Option<(i64, i64)> {
    if fuel == 0 {
        return None;
    }
    if let Operand::Const(c) = o {
        let v = c.to_i64()?;
        return Some((v, v));
    }
    let inst = definition(ctx, o)?;
    match inst {
        Instruction::Copy { src, .. } => return bounds(ctx, src, fuel - 1),
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if ty_is_integer(*from_ty) && ty_is_integer(*to_ty) => {
            if *to_ty == IrType::U8 {
                return Some((0, 255));
            }
            if let Some((lo, hi)) = bounds(ctx, src, fuel - 1) {
                if lo >= 0 && hi <= 255 {
                    return Some((lo, hi));
                }
            }
        }
        Instruction::Load { ty: IrType::U8, .. } => {
            if let Some(src) = forwarded(ctx, o) {
                return bounds(ctx, &src, fuel - 1);
            }
            return Some((0, 255));
        }
        Instruction::Select { .. } => {
            if let Some((max, a, b, unsigned)) = minmax(ctx, o) {
                let (al, ah) = bounds(ctx, &a, fuel - 1)?;
                let (bl, bh) = bounds(ctx, &b, fuel - 1)?;
                // Never use mathematical signed bounds for a bit-reinterpreted
                // predicate whose order is different on a possible operand.
                if (unsigned && (al < 0 || bl < 0))
                    || (!unsigned && (ah > i32::MAX as i64 || bh > i32::MAX as i64))
                {
                    return None;
                }
                return Some(if max {
                    (al.max(bl), ah.max(bh))
                } else {
                    (al.min(bl), ah.min(bh))
                });
            }
        }
        Instruction::BinOp {
            op: IrBinOp::And,
            rhs: Operand::Const(c),
            ..
        } if c.to_i64().is_some_and(|v| (0..=255).contains(&v)) => return Some((0, c.to_i64()?)),
        _ => {}
    }
    let ty = match inst {
        Instruction::Cast { to_ty, .. } => *to_ty,
        Instruction::BinOp { ty, .. }
        | Instruction::Load { ty, .. }
        | Instruction::Select { ty, .. }
        | Instruction::ParamRef { ty, .. } => *ty,
        _ => return None,
    };
    match ty {
        IrType::U32 => Some((0, u32::MAX as i64)),
        IrType::I32 => Some((i32::MIN as i64, i32::MAX as i64)),
        _ => None,
    }
}

fn byte_bounded(ctx: &BlockCtx, o: &Operand, fuel: usize) -> bool {
    bounds(ctx, o, fuel).is_some_and(|(lo, hi)| lo >= 0 && hi <= 255)
}

fn wide_store_source(ctx: &BlockCtx, o: &Operand) -> Option<(Operand, IrType)> {
    let o = strip_copies(ctx, o);
    let Instruction::Cast {
        src,
        from_ty,
        to_ty: IrType::U8,
        ..
    } = definition(ctx, &o)?
    else {
        return None;
    };
    if !matches!(from_ty, IrType::I32 | IrType::U32) || !byte_bounded(ctx, src, 16) {
        return None;
    }
    Some((src.clone(), *from_ty))
}

fn leaves(ctx: &BlockCtx, o: &Operand, out: &mut Vec<Operand>, fuel: usize) -> Option<()> {
    if fuel == 0 || out.len() > 4 {
        return None;
    }
    let o = canonical(ctx, o, 16);
    if let Some((true, a, b, _)) = minmax(ctx, &o) {
        // Only byte-domain maxima are associative in the required order.
        if !byte_bounded(ctx, &a, 16) || !byte_bounded(ctx, &b, 16) {
            return None;
        }
        leaves(ctx, &a, out, fuel - 1)?;
        leaves(ctx, &b, out, fuel - 1)?;
    } else if !out.contains(&o) {
        out.push(o);
    }
    Some(())
}

pub(super) fn seed(ctx: &BlockCtx, run: &[(i64, usize, Operand, Value)]) -> Option<SeedCandidate> {
    if run.len() != 4 || !x86_sse41_available_pub() {
        return None;
    }
    let sources: Vec<_> = run
        .iter()
        .map(|(_, _, o, _)| wide_store_source(ctx, o))
        .collect::<Option<_>>()?;
    let (lane_ops, ty, tail) = find_tail(ctx, &sources)?;
    Some(SeedCandidate {
        store_idx: run.iter().map(|r| r.1).collect(),
        lane_ops,
        ty,
        anchor_ptr: run[0].3,
        width: 4,
        joint: Some(tail),
        value_sink: None,
    })
}

fn find_tail(
    ctx: &BlockCtx,
    sources: &[(Operand, IrType)],
) -> Option<(Vec<Operand>, IrType, Tail)> {
    for alpha in 0..4 {
        let mut reduced = Vec::new();
        leaves(ctx, &sources[alpha].0, &mut reduced, 8)?;
        if !(3..=4).contains(&reduced.len()) {
            continue;
        }
        let others: Vec<_> = (0..4).filter(|&i| i != alpha).collect();
        let mut remaining = reduced.clone();
        let mut matches = true;
        for &i in &others {
            let source = canonical(ctx, &sources[i].0, 16);
            if let Some(pos) = remaining.iter().position(|s| *s == source) {
                remaining.remove(pos);
            } else {
                matches = false;
                break;
            }
        }
        if !matches || remaining.len() > 1 {
            continue;
        }
        let ty = sources[others[0]].1;
        if sources.iter().any(|(_, t)| *t != ty) {
            continue;
        }
        let mut lanes: Vec<_> = sources.iter().map(|(o, _)| o.clone()).collect();
        let four_inputs = !remaining.is_empty();
        lanes[alpha] = if four_inputs {
            remaining.remove(0)
        } else {
            lanes[others[0]].clone()
        };
        return Some((lanes, ty, Tail { alpha, four_inputs }));
    }
    None
}

/// A shape-specific MEMORY endpoint, not a specialized arithmetic idiom:
/// any ordinary SLP graph may widen these three exact bytes. All four
/// logical lanes must name one of the same three byte loads, including a
/// duplicate lane. Reads use 2+1 bytes, never a speculative fourth byte.
pub(super) fn byte_load_pack(ctx: &BlockCtx, vals: &[Value]) -> Option<Pack> {
    if vals.len() != 4 {
        return None;
    }
    let mut raw = Vec::new();
    let mut ptrs = Vec::new();
    let mut addresses = Vec::new();
    for value in vals {
        let Instruction::Cast {
            src: Operand::Value(load),
            from_ty: IrType::U8,
            to_ty: IrType::I32 | IrType::U32,
            ..
        } = definition(ctx, &Operand::Value(*value))?
        else {
            return None;
        };
        let Instruction::Load {
            ptr,
            ty: IrType::U8,
            volatile: false,
            seg_override: AddressSpace::Default,
            ..
        } = definition(ctx, &Operand::Value(*load))?
        else {
            return None;
        };
        raw.push(*load);
        ptrs.push(*ptr);
        addresses.push(eval_sym_addr(ctx.block, &ctx.def_pos, *ptr)?);
    }
    let first = addresses.iter().min_by_key(|a| a.off)?;
    if addresses
        .iter()
        .any(|a| (a.base, a.var, a.mult) != (first.base, first.var, first.mult))
    {
        return None;
    }
    let mut indices = Vec::new();
    for a in &addresses {
        indices.push(a.off.checked_sub(first.off)?);
    }
    let count = indices.iter().max()? + 1;
    if !(3..=4).contains(&count) || !(0..count).all(|i| indices.contains(&i)) {
        return None;
    }
    let anchor = ptrs[indices.iter().position(|i| *i == 0)?];
    let pattern = indices
        .iter()
        .enumerate()
        .fold(0, |v, (i, n)| v | ((*n as i32) << (2 * i)));
    Some(Pack {
        kind: PackKind::MemLoad {
            ptrs,
            offs: addresses.iter().map(|a| a.off).collect(),
            byte3: Some((anchor, pattern, count as u8)),
        },
        lane_vals: raw,
        sched: usize::MAX,
        order: 0,
    })
}

/// Read-after-write check against ALL retired stores, including overwritten
/// stores outside the final sink list. A vector endpoint must not re-read the
/// pre-store bytes where a scalar input originally consumed a retired write.
pub(super) fn removed_stores_preserve_loads(
    ctx: &BlockCtx,
    packs: &[Pack],
    removed: &FxHashSet<usize>,
    bases: &RestrictBases,
    lane_size: u64,
) -> bool {
    for pack in packs {
        let PackKind::MemLoad { ptrs, offs, byte3 } = &pack.kind else {
            continue;
        };
        let size = if byte3.is_some() { 1 } else { lane_size as i64 };
        for (li, value) in pack.lane_vals.iter().enumerate() {
            let pos = ctx.def_pos[&value.0];
            let Some(load) = eval_sym_addr(ctx.block, &ctx.def_pos, ptrs[li]) else {
                return false;
            };
            for &w in removed {
                if w >= pos {
                    continue;
                }
                let Instruction::Store { ptr, ty, .. } = &ctx.block.instructions[w] else {
                    continue;
                };
                let Some(store) = eval_sym_addr(ctx.block, &ctx.def_pos, *ptr) else {
                    return false;
                };
                if (load.base, load.var, load.mult) == (store.base, store.var, store.mult) {
                    if byte_ranges_overlap(offs[li], size, store.off, ty.size() as i64) {
                        return false;
                    }
                } else if !bases.disjoint(load.base.0, store.base.0) {
                    return false;
                }
            }
        }
    }
    true
}

fn packed_fields(
    ctx: &BlockCtx,
    o: &Operand,
    fields: &mut [Option<Operand>; 4],
    fuel: usize,
) -> Option<()> {
    if fuel == 0 {
        return None;
    }
    let o = strip_copies(ctx, o);
    let (slot, source) = if byte_bounded(ctx, &o, 16) {
        (0, o.clone())
    } else if let Some(Instruction::BinOp {
        op: IrBinOp::Shl,
        lhs,
        rhs,
        ty: IrType::U32,
        ..
    }) = definition(ctx, &o)
    {
        let amount = const_amount(ctx, rhs)?;
        if ![8, 16, 24].contains(&amount) || !byte_bounded(ctx, lhs, 16) {
            return None;
        }
        (amount as usize / 8, lhs.clone())
    } else if let Some(Instruction::BinOp {
        op: IrBinOp::Or,
        lhs,
        rhs,
        ty: IrType::U32,
        ..
    }) = definition(ctx, &o)
    {
        packed_fields(ctx, lhs, fields, fuel - 1)?;
        return packed_fields(ctx, rhs, fields, fuel - 1);
    } else {
        return None;
    };
    if fields[slot].replace(source).is_some() {
        return None;
    }
    Some(())
}

pub(super) fn value_seed(ctx: &BlockCtx) -> Option<SeedCandidate> {
    if !x86_sse41_available_pub() {
        return None;
    }
    let Terminator::Return(Some(Operand::Value(value))) = &ctx.block.terminator else {
        return None;
    };
    let mut fields = [None, None, None, None];
    packed_fields(ctx, &Operand::Value(*value), &mut fields, 16)?;
    let sources: Vec<_> = fields
        .into_iter()
        .map(|o| o.map(|v| (v, IrType::U32)))
        .collect::<Option<_>>()?;
    let (lane_ops, ty, tail) = find_tail(ctx, &sources)?;
    let pos = *ctx.def_pos.get(&value.0)?;
    Some(SeedCandidate {
        store_idx: vec![pos; 4],
        lane_ops,
        ty,
        anchor_ptr: *value,
        width: 4,
        joint: Some(tail),
        value_sink: Some(*value),
    })
}

pub(super) fn byte_field_pack(ctx: &BlockCtx, vals: &[Value]) -> Option<Pack> {
    if vals.len() != 4 {
        return None;
    }
    let mut source = None;
    let mut pattern = 0;
    for (lane, value) in vals.iter().enumerate() {
        let Instruction::Cast {
            src: Operand::Value(byte),
            from_ty: IrType::U8,
            to_ty: IrType::I32 | IrType::U32,
            ..
        } = definition(ctx, &Operand::Value(*value))?
        else {
            return None;
        };
        let Instruction::Cast {
            src,
            from_ty: IrType::U32,
            to_ty: IrType::U8,
            ..
        } = definition(ctx, &Operand::Value(*byte))?
        else {
            return None;
        };
        let (packed, shift) = match definition(ctx, src) {
            Some(Instruction::BinOp {
                op: IrBinOp::LShr,
                lhs,
                rhs,
                ty: IrType::U32,
                ..
            }) => (lhs.clone(), const_amount(ctx, rhs)?),
            _ => (src.clone(), 0),
        };
        if ![0, 8, 16, 24].contains(&shift) {
            return None;
        }
        if source.as_ref().is_some_and(|s| *s != packed) {
            return None;
        }
        source = Some(packed);
        pattern |= ((shift / 8) as i32) << (lane * 2);
    }
    Some(Pack {
        kind: PackKind::ByteFields {
            src: source?,
            pattern,
        },
        lane_vals: vals.to_vec(),
        sched: usize::MAX,
        order: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::BlockId;

    #[test]
    fn retired_store_must_not_change_original_vector_inputs() {
        // The overwritten byte store is not one of the four final sinks.
        // Its later read MUST NOT be replaced by pre-store memory.
        let external = FxHashSet::default();
        let steps = FxHashMap::default();
        for store_after in [false, true] {
            for offset in [0, 1, 4] {
                let ptr = Value(1);
                let store_ptr = Value(2);
                let load = Instruction::Load {
                    dest: Value(3),
                    ptr,
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                };
                let store = Instruction::Store {
                    val: Operand::Const(IrConst::I8(42)),
                    ptr: store_ptr,
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                };
                let mut instructions = vec![
                    Instruction::ParamRef {
                        dest: ptr,
                        param_idx: 0,
                        ty: IrType::Ptr,
                    },
                    Instruction::GetElementPtr {
                        dest: store_ptr,
                        base: ptr,
                        offset: Operand::Const(IrConst::I64(offset)),
                        ty: IrType::Ptr,
                    },
                ];
                instructions.extend(if store_after {
                    vec![load, store]
                } else {
                    vec![store, load]
                });
                let block = BasicBlock {
                    label: BlockId(0),
                    instructions,
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                };
                let ctx = BlockCtx {
                    block: &block,
                    def_pos: block
                        .instructions
                        .iter()
                        .enumerate()
                        .filter_map(|(i, inst)| inst.dest().map(|d| (d.0, i)))
                        .collect(),
                    uses: FxHashMap::default(),
                    external_uses: &external,
                    fp_contract: Default::default(),
                    loop_steps: &steps,
                    debug: false,
                };
                let packs = vec![Pack {
                    kind: PackKind::MemLoad {
                        ptrs: vec![ptr; 4],
                        offs: vec![0; 4],
                        byte3: Some((ptr, 0, 3)),
                    },
                    lane_vals: vec![Value(3); 4],
                    sched: 0,
                    order: 0,
                }];
                let removed = FxHashSet::from_iter([if store_after { 3 } else { 2 }]);
                assert_eq!(
                    removed_stores_preserve_loads(
                        &ctx,
                        &packs,
                        &removed,
                        &RestrictBases::default(),
                        4
                    ),
                    store_after || offset != 0
                );
            }
        }
    }

    #[test]
    fn register_tail_has_scalar_metadata_and_no_memory_effect() {
        let op = IntrinsicOp::VecNarrowMaxU8x4;
        assert!(op.is_pure());
        assert!(!op.produces_vector_value());
        assert!(!op.may_read_memory());
        assert!(!op.writes_memory_via_args());
        assert!(op.is_horizontal_vector_reduce());
        assert_eq!(op.vector_home_width(), None);
        assert!(IntrinsicOp::VecWidenByteFieldsI32x4.produces_vector_value());
        assert!(!IntrinsicOp::VecWidenByteFieldsI32x4.may_read_memory());
    }
}
