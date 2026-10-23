//! Demand-driven scalar observations, cancellation and byte-permutation algebra.
//! No producer is repurposed: snapshots are immutable and only consumers change.
//! In particular, simplifying a Select never deletes evaluation of its condition.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Test {
    value: Operand,
    mask: u64,
    constant: u64,
    op: IrCmpOp,
}
pub(super) fn full(ty: IrType) -> Option<u64> {
    match ty {
        IrType::I32 | IrType::U32 => Some(u32::MAX as u64),
        IrType::I64 | IrType::U64 => Some(u64::MAX),
        _ => None,
    }
}
fn view(mut value: Operand, defs: &[Option<Instruction>]) -> Operand {
    for _ in 0..32 {
        let Operand::Value(v) = value else { break };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => value = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if compatible(*from_ty, *to_ty) => value = *src,
            _ => break,
        }
    }
    value
}
pub(super) fn same_bits(a: Operand, b: Operand, defs: &[Option<Instruction>]) -> bool {
    view(a, defs) == view(b, defs)
}
pub(super) fn definition(value: Operand, defs: &[Option<Instruction>]) -> Option<&Instruction> {
    let Operand::Value(v) = view(value, defs) else {
        return None;
    };
    defs.get(v.0 as usize)?.as_ref()
}
pub(super) fn compatible(a: IrType, b: IrType) -> bool {
    full(a).is_some() && full(b).is_some() && a.size() == b.size()
}
fn inverse_odd(n: u64) -> u64 {
    // Newton/Hensel lifting doubles the correct low bits each iteration.
    let mut inverse = 1u64;
    for _ in 0..6 {
        inverse = inverse.wrapping_mul(2u64.wrapping_sub(n.wrapping_mul(inverse)));
    }
    inverse
}
pub(super) fn swap(n: u64, ty: IrType) -> u64 {
    if ty.size() == 4 {
        (n as u32).swap_bytes() as u64
    } else {
        n.swap_bytes()
    }
}
pub(super) fn rotate(n: u64, amount: u32, ty: IrType, left: bool) -> u64 {
    if ty.size() == 4 {
        if left {
            (n as u32).rotate_left(amount) as u64
        } else {
            (n as u32).rotate_right(amount) as u64
        }
    } else if left {
        n.rotate_left(amount)
    } else {
        n.rotate_right(amount)
    }
}
fn negated(x: Operand, root: Operand, ty: IrType, defs: &[Option<Instruction>]) -> bool {
    matches!(definition(x, defs), Some(Instruction::UnaryOp { op:IrUnaryOp::Neg, src,ty:t,.. }) if compatible(*t,ty) && same_bits(*src,root,defs))
        || matches!(definition(x,defs),Some(Instruction::BinOp { op:IrBinOp::Sub,lhs,rhs,ty:t,.. }) if compatible(*t,ty) && const_in_ring(*lhs,ty.size()*8)==Some(0) && same_bits(*rhs,root,defs))
}

/// Use counts over operands and terminators (the cost model of [`observe`]).
fn use_counts(func: &IrFunction) -> Vec<u32> {
    let mut uses = vec![0u32; func.max_value_id() as usize + 1];
    let mut count = |v: &Value| {
        if let Some(n) = uses.get_mut(v.0 as usize) {
            *n += 1;
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    count(v);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, &mut count);
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                count(v);
            }
        });
    }
    uses
}

/// True when `value` and every copy/compatible cast [`view`] peels on the
/// way to its producer have exactly one use: the producer dies with the test.
fn dies_with_test(mut value: Operand, defs: &[Option<Instruction>], uses: &[u32]) -> bool {
    for _ in 0..32 {
        let Operand::Value(v) = value else {
            return true;
        };
        if uses.get(v.0 as usize).copied() != Some(1) {
            return false;
        }
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => value = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if compatible(*from_ty, *to_ty) => value = *src,
            _ => return true,
        }
    }
    false
}

/// Producers whose surviving copy makes a peel worth at most one cycle of
/// latency: walking through such a SHARED producer saves no instruction (it
/// still executes for its other users) but extends its root's live range.
/// Multi-cycle producers (Mul, UDiv, counts, byte/bit reversal) still pay
/// for the peel by taking the compare off their latency chain.
fn single_cycle_producer(inst: Option<&Instruction>) -> bool {
    match inst {
        Some(Instruction::BinOp { op, .. }) => matches!(
            op,
            IrBinOp::Add
                | IrBinOp::Sub
                | IrBinOp::And
                | IrBinOp::Or
                | IrBinOp::Xor
                | IrBinOp::Shl
                | IrBinOp::LShr
                | IrBinOp::AShr
                | IrBinOp::RotateLeft
                | IrBinOp::RotateRight
        ),
        Some(Instruction::UnaryOp { op, .. }) => matches!(op, IrUnaryOp::Not | IrUnaryOp::Neg),
        Some(Instruction::Select { .. }) => true,
        _ => false,
    }
}

/// Walk `test` back through invertible producers. `shared` is raised when
/// the returned test was obtained by peeling a single-cycle producer that
/// outlives the test (see [`single_cycle_producer`]); the caller then keeps
/// only rewrites that are cheaper than the compare itself. A level counts
/// as peeled only if its result names a different root, so an abandoned
/// sub-walk (a Select whose arms disagree) cannot veto anything.
fn observe(
    test: Test,
    ty: IrType,
    defs: &[Option<Instruction>],
    uses: &[u32],
    depth: u8,
    budget: &mut u8,
    shared: &mut bool,
) -> Test {
    let root = view(test.value, defs);
    let crossing_shared =
        !dies_with_test(test.value, defs, uses) && single_cycle_producer(definition(root, defs));
    let mut below = false;
    let result = observe_level(test, ty, defs, uses, depth, budget, &mut below);
    if result.value != root {
        *shared |= crossing_shared || below;
    }
    result
}

fn observe_level(
    mut test: Test,
    ty: IrType,
    defs: &[Option<Instruction>],
    uses: &[u32],
    depth: u8,
    budget: &mut u8,
    shared: &mut bool,
) -> Test {
    if depth == 0 || *budget == 0 {
        return test;
    }
    *budget -= 1;
    let Some(all) = full(ty) else {
        return test;
    };
    let width = ty.size() * 8;
    test.value = view(test.value, defs);
    let mut recurse = |value, mask, constant, op| {
        observe(
            Test {
                value,
                mask: mask & all,
                constant: constant & all,
                op,
            },
            ty,
            defs,
            uses,
            depth - 1,
            budget,
            shared,
        )
    };
    match definition(test.value, defs) {
        Some(Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        }) if compatible(*from_ty, ty) && compatible(*to_ty, ty) => {
            recurse(*src, test.mask, test.constant, test.op)
        }
        Some(Instruction::Select {
            true_val,
            false_val,
            ty: t,
            ..
        }) if compatible(*t, ty) => {
            let a = recurse(*true_val, test.mask, test.constant, test.op);
            let b = recurse(*false_val, test.mask, test.constant, test.op);
            if a == b { a } else { test }
        }
        Some(Instruction::UnaryOp { op, src, ty: t, .. }) if compatible(*t, ty) => match op {
            IrUnaryOp::Not => recurse(*src, test.mask, test.constant ^ test.mask, test.op),
            IrUnaryOp::Neg if test.mask == all => {
                recurse(*src, all, test.constant.wrapping_neg(), test.op)
            }
            IrUnaryOp::Bswap => {
                recurse(*src, swap(test.mask, ty), swap(test.constant, ty), test.op)
            }
            IrUnaryOp::BitReverse => {
                let rev = |n: u64| n.reverse_bits() >> (64 - width);
                recurse(*src, rev(test.mask), rev(test.constant), test.op)
            }
            IrUnaryOp::Popcount if test.mask == all && test.constant == 0 => {
                recurse(*src, all, 0, test.op)
            }
            IrUnaryOp::Popcount if test.mask == all && test.constant == width as u64 => {
                recurse(*src, all, all, test.op)
            }
            IrUnaryOp::Clz | IrUnaryOp::Ctz
                if test.mask == all && test.constant == width as u64 =>
            {
                recurse(*src, all, 0, test.op)
            }
            IrUnaryOp::Clz | IrUnaryOp::ClzNonZero if test.mask == all && test.constant == 0 => {
                recurse(*src, 1u64 << (width - 1), 1u64 << (width - 1), test.op)
            }
            IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero if test.mask == all && test.constant == 0 => {
                recurse(*src, 1, 1, test.op)
            }
            _ => test,
        },
        Some(Instruction::BinOp {
            op,
            lhs,
            rhs,
            ty: t,
            ..
        }) if compatible(*t, ty) => {
            let constant = const_in_ring(*rhs, width)
                .map(|c| (*lhs, c, false))
                .or_else(|| const_in_ring(*lhs, width).map(|c| (*rhs, c, true)));
            if let Some((x, c, reversed)) = constant {
                match op {
                    IrBinOp::And if test.constant & !c == 0 => {
                        return recurse(x, test.mask & c, test.constant, test.op);
                    }
                    IrBinOp::Or if test.constant & c == test.mask & c => {
                        return recurse(x, test.mask & !c, test.constant & !c, test.op);
                    }
                    IrBinOp::Xor => {
                        return recurse(x, test.mask, test.constant ^ (c & test.mask), test.op);
                    }
                    IrBinOp::Add if test.mask == all => {
                        return recurse(x, all, test.constant.wrapping_sub(c), test.op);
                    }
                    IrBinOp::Sub if test.mask == all => {
                        return recurse(
                            x,
                            all,
                            if reversed {
                                c.wrapping_sub(test.constant)
                            } else {
                                test.constant.wrapping_add(c)
                            },
                            test.op,
                        );
                    }
                    IrBinOp::Mul if test.mask == all && c & 1 == 1 => {
                        return recurse(
                            x,
                            all,
                            test.constant.wrapping_mul(inverse_odd(c)),
                            test.op,
                        );
                    }
                    IrBinOp::RotateLeft | IrBinOp::RotateRight if !reversed => {
                        return recurse(
                            x,
                            rotate(test.mask, c as u32, ty, *op == IrBinOp::RotateRight),
                            rotate(test.constant, c as u32, ty, *op == IrBinOp::RotateRight),
                            test.op,
                        );
                    }
                    IrBinOp::LShr | IrBinOp::AShr
                        if !reversed && c > 0 && c < (width as u64) && test.mask == all =>
                    {
                        let mask = all << c & all;
                        if test.constant == 0 {
                            return recurse(x, mask, 0, test.op);
                        }
                        if (*op == IrBinOp::LShr && test.constant == all >> c)
                            || (*op == IrBinOp::AShr && test.constant == all)
                        {
                            return recurse(x, mask, mask, test.op);
                        }
                    }
                    IrBinOp::Shl
                        if !reversed
                            && c > 0
                            && c < (width as u64)
                            && test.mask == all
                            && test.constant == 0 =>
                    {
                        return recurse(x, all >> c, 0, test.op);
                    }
                    IrBinOp::UDiv
                        if !reversed && c > 0 && test.mask == all && test.constant == 0 =>
                    {
                        return Test {
                            value: x,
                            mask: all,
                            constant: c,
                            op: if test.op == IrCmpOp::Eq {
                                IrCmpOp::Ult
                            } else {
                                IrCmpOp::Uge
                            },
                        };
                    }
                    _ => (),
                }
            }
            if test.mask == all && test.constant == 0 {
                // Zero-reflecting rings/triangular maps. A positive shift is
                // essential for XOR and odd-factor Add/Sub; shift zero fails.
                for (x, other) in [(*lhs, *rhs), (*rhs, *lhs)] {
                    if matches!(op, IrBinOp::And | IrBinOp::Or) && negated(other, x, ty, defs) {
                        return recurse(x, all, 0, test.op);
                    }
                    if let Some(Instruction::BinOp {
                        op: s,
                        lhs: root,
                        rhs: amount,
                        ty: st,
                        ..
                    }) = definition(other, defs)
                    {
                        if compatible(*st, ty) && same_bits(x, *root, defs) {
                            if let Some(k) = const_in_ring(*amount, width) {
                                if k > 0
                                    && k < (width as u64)
                                    && (matches!(op, IrBinOp::Or | IrBinOp::Xor)
                                        && matches!(s, IrBinOp::Shl | IrBinOp::LShr)
                                        || matches!(op, IrBinOp::Add | IrBinOp::Sub)
                                            && *s == IrBinOp::Shl)
                                {
                                    return recurse(x, all, 0, test.op);
                                }
                            }
                        }
                    }
                }
            }
            test
        }
        _ => test,
    }
}

fn cancel(
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand)> {
    match (definition(lhs, defs)?, definition(rhs, defs)?) {
        (
            Instruction::UnaryOp {
                op: a,
                src: x,
                ty: ta,
                ..
            },
            Instruction::UnaryOp {
                op: b,
                src: y,
                ty: tb,
                ..
            },
        ) if a == b
            && compatible(*ta, ty)
            && compatible(*tb, ty)
            && matches!(
                a,
                IrUnaryOp::Neg | IrUnaryOp::Not | IrUnaryOp::Bswap | IrUnaryOp::BitReverse
            ) =>
        {
            Some((*x, *y))
        }
        (
            Instruction::BinOp {
                op: a,
                lhs: x,
                rhs: k,
                ty: ta,
                ..
            },
            Instruction::BinOp {
                op: b,
                lhs: y,
                rhs: l,
                ty: tb,
                ..
            },
        ) if a == b && compatible(*ta, ty) && compatible(*tb, ty) => {
            if same_bits(*k, *l, defs)
                && matches!(
                    a,
                    IrBinOp::Add
                        | IrBinOp::Sub
                        | IrBinOp::Xor
                        | IrBinOp::RotateLeft
                        | IrBinOp::RotateRight
                )
            {
                return Some((*x, *y));
            }
            if same_bits(*x, *y, defs) && matches!(a, IrBinOp::Add | IrBinOp::Sub | IrBinOp::Xor) {
                return Some((*k, *l));
            }
            if matches!(a, IrBinOp::Add | IrBinOp::Xor) {
                if same_bits(*x, *l, defs) {
                    return Some((*k, *y));
                }
                if same_bits(*k, *y, defs) {
                    return Some((*x, *l));
                }
            }
            None
        }
        _ => None,
    }
}
fn unswap(value: Operand, ty: IrType, defs: &[Option<Instruction>]) -> Option<Operand> {
    if let Operand::Const(c) = value {
        return const_in_ring(Operand::Const(c), ty.size() * 8)
            .map(|n| Operand::Const(IrConst::from_i64(swap(n, ty) as i64, ty)));
    }
    match definition(value, defs)? {
        Instruction::UnaryOp {
            op: IrUnaryOp::Bswap,
            src,
            ty: t,
            ..
        } if compatible(*t, ty) => Some(*src),
        _ => None,
    }
}
fn conjugate(inst: &Instruction, defs: &[Option<Instruction>]) -> Option<Instruction> {
    let Instruction::UnaryOp {
        dest,
        op: IrUnaryOp::Bswap,
        src,
        ty,
    } = inst
    else {
        return None;
    };
    full(*ty)?;
    match definition(*src, defs)? {
        Instruction::UnaryOp {
            op: IrUnaryOp::Not,
            src,
            ty: t,
            ..
        } if compatible(*t, *ty) => Some(Instruction::UnaryOp {
            dest: *dest,
            op: IrUnaryOp::Not,
            src: unswap(*src, *ty, defs)?,
            ty: *ty,
        }),
        Instruction::BinOp {
            op,
            lhs,
            rhs,
            ty: t,
            ..
        } if compatible(*t, *ty) => {
            let (new_op, right) = match op {
                IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => (*op, unswap(*rhs, *ty, defs)?),
                IrBinOp::Shl | IrBinOp::LShr | IrBinOp::RotateLeft | IrBinOp::RotateRight => {
                    let k = const_in_ring(*rhs, ty.size() * 8)?;
                    if k == 0 || k >= ty.size() as u64 * 8 || !k.is_multiple_of(8) {
                        return None;
                    }
                    (
                        match op {
                            IrBinOp::Shl => IrBinOp::LShr,
                            IrBinOp::LShr => IrBinOp::Shl,
                            IrBinOp::RotateLeft => IrBinOp::RotateRight,
                            _ => IrBinOp::RotateLeft,
                        },
                        *rhs,
                    )
                }
                _ => return None,
            };
            Some(Instruction::BinOp {
                dest: *dest,
                op: new_op,
                lhs: unswap(*lhs, *ty, defs)?,
                rhs: right,
                ty: *ty,
            })
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ty: t,
            ..
        } if compatible(*t, *ty) => Some(Instruction::Select {
            dest: *dest,
            cond: *cond,
            true_val: unswap(*true_val, *ty, defs)?,
            false_val: unswap(*false_val, *ty, defs)?,
            ty: *ty,
        }),
        _ => None,
    }
}

pub(super) fn run(func: &mut IrFunction, defs: &[Option<Instruction>]) -> usize {
    let mut changes = 0;
    // Snapshot, like `defs`: consumers are rewritten in place below, which
    // never changes any producer's use count before it is consulted.
    let uses = use_counts(func);
    let mut next = func
        .max_value_id()
        .saturating_add(1)
        .max(func.next_value_id);
    for block in &mut func.blocks {
        let old = std::mem::take(&mut block.instructions);
        let spans = std::mem::take(&mut block.source_spans);
        for (index, inst) in old.into_iter().enumerate() {
            let span = spans
                .get(index)
                .copied()
                .unwrap_or_else(crate::common::source::Span::dummy);
            let mut replacement = conjugate(&inst, defs);
            let mut prefix = None;
            if let Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = &inst
            {
                if let Some(all) = full(*ty) {
                    if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
                        // Cancelling two producers that both outlive the
                        // compare and cost a cycle each saves nothing and
                        // extends both roots' live ranges.
                        let pays = |o: Operand| {
                            dies_with_test(o, defs, &uses)
                                || !single_cycle_producer(definition(o, defs))
                        };
                        if let Some((a, b)) =
                            cancel(*lhs, *rhs, *ty, defs).filter(|_| pays(*lhs) || pays(*rhs))
                        {
                            replacement = Some(Instruction::Cmp {
                                dest: *dest,
                                op: *op,
                                lhs: a,
                                rhs: b,
                                ty: *ty,
                            });
                        }
                        let pair = const_in_ring(*rhs, ty.size() * 8)
                            .map(|c| (*lhs, c))
                            .or_else(|| const_in_ring(*lhs, ty.size() * 8).map(|c| (*rhs, c)));
                        if let Some((value, constant)) = pair {
                            let original = Test {
                                value,
                                mask: all,
                                constant,
                                op: *op,
                            };
                            let mut shared = false;
                            let mut test =
                                observe(original, *ty, defs, &uses, 12, &mut 64, &mut shared);
                            // Materialize already-normalized single-bit booleans
                            // directly. At 32 bits this preserves Cmp's I32 result
                            // width and avoids SETcc plus a zero-extension. Do
                            // not silently change a 64-bit input/result width.
                            if ty.size() == 4
                                && ((test.op == IrCmpOp::Eq && test.constant == test.mask)
                                    || (test.op == IrCmpOp::Ne && test.constant == 0))
                            {
                                let direct = if test.mask == 1 {
                                    Some((IrBinOp::And, 1))
                                } else if test.mask == 1u64 << 31 {
                                    Some((IrBinOp::LShr, 31))
                                } else {
                                    None
                                };
                                if let Some((op, k)) = direct {
                                    block.instructions.push(Instruction::BinOp {
                                        dest: *dest,
                                        op,
                                        lhs: test.value,
                                        rhs: Operand::Const(IrConst::I32(k)),
                                        ty: IrType::I32,
                                    });
                                    if !spans.is_empty() {
                                        block.source_spans.push(span);
                                    }
                                    changes += 1;
                                    continue;
                                }
                            }
                            if test.mask != all
                                && test.mask != 0
                                && matches!(test.op, IrCmpOp::Eq | IrCmpOp::Ne)
                            {
                                let low = all ^ test.mask;
                                if low & low.wrapping_add(1) == 0
                                    && (test.constant == 0 || test.constant == test.mask)
                                {
                                    let upper = test.constant == test.mask;
                                    test.constant = if upper { test.mask } else { low + 1 };
                                    test.mask = all;
                                    test.op = if upper == (test.op == IrCmpOp::Eq) {
                                        IrCmpOp::Uge
                                    } else {
                                        IrCmpOp::Ult
                                    };
                                } else if test.mask.is_power_of_two() && test.constant == test.mask
                                {
                                    test.constant = 0;
                                    test.op = if test.op == IrCmpOp::Eq {
                                        IrCmpOp::Ne
                                    } else {
                                        IrCmpOp::Eq
                                    };
                                }
                            }
                            // Past the direct materialization above, a test
                            // reached through a shared single-cycle producer
                            // is no cheaper than the original compare.
                            if !shared
                                && test != original
                                && !(test.mask == original.mask
                                    && test.constant == original.constant
                                    && test.op == original.op
                                    && same_bits(test.value, original.value, defs))
                            {
                                // Reuse an already-canonical masked operand. In
                                // particular, do not mint a new AND every pass
                                // iteration for the identical masked test.
                                let canonical_mask = match definition(value, defs) {
                                    Some(Instruction::BinOp {
                                        op: IrBinOp::And,
                                        lhs: a,
                                        rhs: b,
                                        ty: t,
                                        ..
                                    }) if compatible(*t, *ty) => {
                                        (same_bits(*a, test.value, defs)
                                            && const_in_ring(*b, ty.size() * 8) == Some(test.mask))
                                            || (same_bits(*b, test.value, defs)
                                                && const_in_ring(*a, ty.size() * 8)
                                                    == Some(test.mask))
                                    }
                                    _ => false,
                                };
                                if canonical_mask && test.constant == constant && test.op == *op {
                                    block.instructions.push(inst);
                                    if !spans.is_empty() {
                                        block.source_spans.push(span);
                                    }
                                    continue;
                                }
                                let mut operand = if canonical_mask { value } else { test.value };
                                if test.mask != all && !canonical_mask && next < u32::MAX {
                                    let v = Value(next);
                                    next += 1;
                                    prefix = Some(Instruction::BinOp {
                                        dest: v,
                                        op: IrBinOp::And,
                                        lhs: operand,
                                        rhs: Operand::Const(IrConst::from_i64(
                                            test.mask as i64,
                                            *ty,
                                        )),
                                        ty: *ty,
                                    });
                                    operand = Operand::Value(v);
                                }
                                if test.mask == all || canonical_mask || prefix.is_some() {
                                    replacement = Some(Instruction::Cmp {
                                        dest: *dest,
                                        op: test.op,
                                        lhs: operand,
                                        rhs: Operand::Const(IrConst::from_i64(
                                            test.constant as i64,
                                            *ty,
                                        )),
                                        ty: *ty,
                                    });
                                }
                            }
                        }
                    }
                    // ctpop(x) <= x for unsigned x, without a range guess, in
                    // either operand order (`x >= ctpop(x)` is the same fact).
                    let oriented = match op {
                        IrCmpOp::Ule | IrCmpOp::Ugt => Some((*lhs, *rhs, *op == IrCmpOp::Ule)),
                        IrCmpOp::Uge | IrCmpOp::Ult => Some((*rhs, *lhs, *op == IrCmpOp::Uge)),
                        _ => None,
                    };
                    if let Some((
                        Instruction::UnaryOp {
                            op: IrUnaryOp::Popcount,
                            src,
                            ty: t,
                            ..
                        },
                        other,
                        holds,
                    )) = oriented.and_then(|(pop, other, holds)| {
                        Some((definition(pop, defs)?, other, holds))
                    }) {
                        if compatible(*t, *ty) && same_bits(*src, other, defs) {
                            replacement = Some(Instruction::Copy {
                                dest: *dest,
                                src: Operand::Const(IrConst::I32(i32::from(holds))),
                            });
                        }
                    }
                }
            }
            // Demand only remainder-invariant low bits, including non-power
            // of two divisors: 2^k must divide d for all demanded bits below k.
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
            } = &inst
            {
                if full(*ty).is_some() {
                    for (value, mask) in [(*lhs, *rhs), (*rhs, *lhs)] {
                        if let Some(m) = const_in_ring(mask, ty.size() * 8) {
                            if let Some(Instruction::BinOp {
                                op: IrBinOp::URem,
                                lhs: x,
                                rhs: d,
                                ty: t,
                                ..
                            }) = definition(value, defs)
                            {
                                if compatible(*t, *ty) {
                                    if let Some(d) = const_in_ring(*d, ty.size() * 8) {
                                        if d != 0 && d.trailing_zeros() >= 64 - m.leading_zeros() {
                                            replacement = Some(Instruction::BinOp {
                                                dest: *dest,
                                                op: IrBinOp::And,
                                                lhs: *x,
                                                rhs: mask,
                                                ty: *ty,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(new) = replacement {
                if let Some(p) = prefix {
                    block.instructions.push(p);
                    if !spans.is_empty() {
                        block.source_spans.push(span);
                    }
                }
                block.instructions.push(new);
                changes += 1;
            } else {
                block.instructions.push(inst);
            }
            if !spans.is_empty() {
                block.source_spans.push(span);
            }
        }
    }
    func.next_value_id = next;
    changes
}

/// Preserve div/rem semantics before strength reduction obscures the producer.
/// Functions without these operations incur only a predicate scan, no maps.
pub(super) fn early(func: &mut IrFunction) -> usize {
    if !func.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::UDiv | IrBinOp::URem,
                ..
            }
        )
    }) {
        return 0;
    }
    let mut defs = vec![None; func.max_value_id() as usize + 1];
    let mut counts = vec![0u32; defs.len()];
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Some(dest) = inst.dest() {
            defs[dest.0 as usize] = Some(inst.clone());
            counts[dest.0 as usize] += 1;
        }
    }
    for (d, c) in defs.iter_mut().zip(counts) {
        if c != 1 {
            *d = None;
        }
    }
    run(func, &defs)
}

#[cfg(test)]
#[path = "bit_idioms_observe_tests.rs"]
mod tests;
