//! Bounded demanded-bit reconstruction and exact bit-count identities.
//! Reconstructed paths must have one use: do not duplicate shared arithmetic.
//! Original producers retain their values; only fresh private SSA nodes may
//! compute partially demanded values. Division by zero and invalid shifts fail.
use super::observe::{compatible, definition, full, rotate, same_bits, swap};
use super::*;

#[derive(Clone)]
enum Expr {
    Leaf(Operand),
    Bin(IrBinOp, Box<Expr>, Box<Expr>),
    Unary(IrUnaryOp, Box<Expr>),
}
struct Plan {
    expr: Expr,
    removed: usize,
}
impl Plan {
    fn unchanged(v: Operand) -> Self {
        Self {
            expr: Expr::Leaf(v),
            removed: 0,
        }
    }
}
fn low_closure(mask: u64) -> u64 {
    if mask == 0 {
        0
    } else {
        u64::MAX >> mask.leading_zeros()
    }
}

fn exclusive(mut v: Operand, defs: &[Option<Instruction>], uses: &[u32]) -> Option<Operand> {
    for _ in 0..32 {
        let Operand::Value(id) = v else {
            return Some(v);
        };
        if uses.get(id.0 as usize) != Some(&1) {
            return None;
        }
        match defs.get(id.0 as usize)?.as_ref()? {
            Instruction::Copy { src, .. } => v = *src,
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if compatible(*from_ty, *to_ty) => v = *src,
            _ => return Some(v),
        }
    }
    None
}
fn private_count_with_uses(
    mut v: Operand,
    defs: &[Option<Instruction>],
    uses: &[u32],
    mut expected: u32,
) -> bool {
    for _ in 0..16 {
        let Operand::Value(id) = v else { return false };
        if uses.get(id.0 as usize) != Some(&expected) {
            return false;
        }
        expected = 1;
        match defs.get(id.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => v = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if full(*from_ty).is_some() && full(*to_ty).is_some() => v = *src,
            Some(Instruction::UnaryOp {
                op: IrUnaryOp::Popcount,
                ..
            }) => return true,
            _ => return false,
        }
    }
    false
}
fn private_count(v: Operand, defs: &[Option<Instruction>], uses: &[u32]) -> bool {
    private_count_with_uses(v, defs, uses, 1)
}
fn demanded(
    v: Operand,
    mask: u64,
    ty: IrType,
    defs: &[Option<Instruction>],
    uses: &[u32],
    budget: &mut u8,
) -> Plan {
    if *budget == 0 {
        return Plan::unchanged(v);
    }
    *budget -= 1;
    if mask == 0 {
        return if const_in_ring(v, ty.size() * 8) == Some(0) {
            Plan::unchanged(v)
        } else {
            Plan {
                expr: Expr::Leaf(Operand::Const(IrConst::from_i64(0, ty))),
                removed: 1,
            }
        };
    }
    let Some(root) = exclusive(v, defs, uses) else {
        return Plan::unchanged(v);
    };
    let Operand::Value(id) = root else {
        return Plan::unchanged(v);
    };
    if uses.get(id.0 as usize) != Some(&1) {
        return Plan::unchanged(v);
    }
    let width = ty.size() * 8;
    let all = full(ty).unwrap();
    let Some(inst) = definition(root, defs) else {
        return Plan::unchanged(v);
    };
    match inst {
        Instruction::BinOp {
            op,
            lhs,
            rhs,
            ty: t,
            ..
        } if compatible(*t, ty) => {
            let constant = const_in_ring(*rhs, width).map(|c| (*lhs, c)).or_else(|| {
                if matches!(op, IrBinOp::And | IrBinOp::Mul) {
                    const_in_ring(*lhs, width).map(|c| (*rhs, c))
                } else {
                    None
                }
            });
            if let Some((x, c)) = constant {
                if (*op == IrBinOp::And && mask & !c == 0)
                    || (*op == IrBinOp::URem
                        && c != 0
                        && c.trailing_zeros() >= 64 - mask.leading_zeros())
                {
                    let mut p = demanded(x, mask, ty, defs, uses, budget);
                    p.removed += 1;
                    return p;
                }
            }
            let (lm, rm) = match op {
                IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => (mask, mask),
                IrBinOp::Add | IrBinOp::Sub => (low_closure(mask), low_closure(mask)),
                IrBinOp::Mul => {
                    if let Some((x, c)) = constant {
                        if c == 0 {
                            return Plan::unchanged(v);
                        }
                        let needed = low_closure(mask) >> c.trailing_zeros();
                        let p = demanded(x, needed, ty, defs, uses, budget);
                        if p.removed > 0 {
                            return Plan {
                                expr: Expr::Bin(
                                    *op,
                                    Box::new(p.expr),
                                    Box::new(Expr::Leaf(Operand::Const(IrConst::from_i64(
                                        c as i64, ty,
                                    )))),
                                ),
                                removed: p.removed,
                            };
                        }
                        return Plan::unchanged(v);
                    }
                    (low_closure(mask), low_closure(mask))
                }
                IrBinOp::Shl
                | IrBinOp::LShr
                | IrBinOp::AShr
                | IrBinOp::RotateLeft
                | IrBinOp::RotateRight => {
                    let Some(k) = const_in_ring(*rhs, width) else {
                        return Plan::unchanged(v);
                    };
                    let m = match op {
                        IrBinOp::RotateLeft | IrBinOp::RotateRight => {
                            rotate(mask, k as u32, ty, *op == IrBinOp::RotateRight)
                        }
                        _ if k >= width as u64 => return Plan::unchanged(v),
                        IrBinOp::Shl => mask >> k,
                        IrBinOp::LShr => mask << k & all,
                        _ => {
                            ((mask << k) & all)
                                | if mask & (all ^ (all >> k)) != 0 {
                                    1u64 << (width - 1)
                                } else {
                                    0
                                }
                        }
                    };
                    (m, all)
                }
                _ => return Plan::unchanged(v),
            };
            let a = demanded(*lhs, lm, ty, defs, uses, budget);
            let b = demanded(*rhs, rm, ty, defs, uses, budget);
            if a.removed + b.removed == 0 {
                Plan::unchanged(v)
            } else {
                Plan {
                    expr: Expr::Bin(*op, Box::new(a.expr), Box::new(b.expr)),
                    removed: a.removed + b.removed,
                }
            }
        }
        Instruction::UnaryOp { op, src, ty: t, .. } if compatible(*t, ty) => {
            let m = match op {
                IrUnaryOp::Not => mask,
                IrUnaryOp::Neg => low_closure(mask),
                IrUnaryOp::Bswap => swap(mask, ty),
                IrUnaryOp::BitReverse => mask.reverse_bits() >> (64 - width),
                _ => return Plan::unchanged(v),
            };
            let p = demanded(*src, m, ty, defs, uses, budget);
            if p.removed == 0 {
                Plan::unchanged(v)
            } else {
                Plan {
                    expr: Expr::Unary(*op, Box::new(p.expr)),
                    removed: p.removed,
                }
            }
        }
        _ => Plan::unchanged(v),
    }
}
fn node_count(e: &Expr) -> u32 {
    match e {
        Expr::Leaf(_) => 0,
        Expr::Unary(_, a) => 1 + node_count(a),
        Expr::Bin(_, a, b) => 1 + node_count(a) + node_count(b),
    }
}
fn emit(e: Expr, ty: IrType, next: &mut u32, out: &mut Vec<Instruction>) -> Operand {
    match e {
        Expr::Leaf(v) => v,
        Expr::Unary(op, a) => {
            let src = emit(*a, ty, next, out);
            let dest = Value(*next);
            *next += 1;
            out.push(Instruction::UnaryOp { dest, op, src, ty });
            Operand::Value(dest)
        }
        Expr::Bin(op, a, b) => {
            let lhs = emit(*a, ty, next, out);
            let rhs = emit(*b, ty, next, out);
            let zero = |v| const_in_ring(v, ty.size() * 8) == Some(0);
            if zero(lhs)
                && matches!(
                    op,
                    IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr | IrBinOp::And | IrBinOp::Mul
                )
            {
                return lhs;
            }
            if zero(rhs) && matches!(op, IrBinOp::And | IrBinOp::Mul) {
                return rhs;
            }
            if zero(lhs) && matches!(op, IrBinOp::Or | IrBinOp::Xor | IrBinOp::Add) {
                return rhs;
            }
            if zero(rhs) && matches!(op, IrBinOp::Or | IrBinOp::Xor | IrBinOp::Add | IrBinOp::Sub) {
                return lhs;
            }
            let dest = Value(*next);
            *next += 1;
            out.push(Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            });
            Operand::Value(dest)
        }
    }
}
fn count_input(mut v: Operand, defs: &[Option<Instruction>]) -> Option<(Operand, IrType)> {
    // Every integer cast preserves bit zero of a count result. Count values
    // themselves fit in 0..64; require at least 32 result bits here.
    for _ in 0..16 {
        if let Operand::Value(id) = v {
            match defs.get(id.0 as usize)?.as_ref()? {
                Instruction::Copy { src, .. } => v = *src,
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if full(*from_ty).is_some() && full(*to_ty).is_some() => v = *src,
                Instruction::UnaryOp {
                    op: IrUnaryOp::Popcount,
                    src,
                    ty,
                    ..
                } if full(*ty).is_some() => return Some((*src, *ty)),
                _ => return None,
            }
        } else {
            return None;
        }
    }
    None
}
fn unary_of(
    v: Operand,
    x: Operand,
    op: IrUnaryOp,
    ty: IrType,
    defs: &[Option<Instruction>],
) -> bool {
    match definition(v, defs) {
        Some(Instruction::UnaryOp {
            op: actual,
            src,
            ty: t,
            ..
        }) => *actual == op && compatible(*t, ty) && same_bits(*src, x, defs),
        _ => false,
    }
}
fn complements(a: Operand, b: Operand, ty: IrType, defs: &[Option<Instruction>]) -> bool {
    const_in_ring(a, ty.size() * 8)
        .zip(const_in_ring(b, ty.size() * 8))
        .is_some_and(|(x, y)| x ^ y == full(ty).unwrap())
        || unary_of(a, b, IrUnaryOp::Not, ty, defs)
        || unary_of(b, a, IrUnaryOp::Not, ty, defs)
}
fn odd(v: Operand, ty: IrType, defs: &[Option<Instruction>]) -> bool {
    let is_odd = |v| const_in_ring(v, ty.size() * 8).is_some_and(|c| c & 1 != 0);
    if is_odd(v) {
        return true;
    }
    match definition(v, defs) {
        Some(Instruction::BinOp {
            op: IrBinOp::Or,
            lhs,
            rhs,
            ty: t,
            ..
        }) => compatible(*t, ty) && (is_odd(*lhs) || is_odd(*rhs)),
        _ => false,
    }
}
fn inverse_neg(v: Operand, x: Operand, ty: IrType, defs: &[Option<Instruction>]) -> bool {
    if unary_of(v, x, IrUnaryOp::Neg, ty, defs) {
        return true;
    }
    match definition(v, defs) {
        Some(Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ty: t,
            ..
        }) => {
            compatible(*t, ty)
                && const_in_ring(*lhs, ty.size() * 8) == Some(0)
                && same_bits(*rhs, x, defs)
        }
        _ => false,
    }
}
fn count_source_once(
    op: IrUnaryOp,
    v: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    match definition(v, defs)? {
        Instruction::UnaryOp {
            op: p, src, ty: t, ..
        } if compatible(*t, ty) => match (op, p) {
            (IrUnaryOp::Popcount, IrUnaryOp::Bswap | IrUnaryOp::BitReverse) => Some(*src),
            (IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero, IrUnaryOp::Neg) => Some(*src),
            _ => None,
        },
        Instruction::BinOp {
            op: b,
            lhs,
            rhs,
            ty: t,
            ..
        } if compatible(*t, ty) => {
            if op == IrUnaryOp::Popcount && matches!(b, IrBinOp::RotateLeft | IrBinOp::RotateRight)
            {
                return Some(*lhs);
            }
            let trailing = matches!(op, IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero);
            let leading = matches!(op, IrUnaryOp::Clz | IrUnaryOp::ClzNonZero);
            if trailing && *b == IrBinOp::Mul {
                if odd(*rhs, ty, defs) {
                    return Some(*lhs);
                }
                if odd(*lhs, ty, defs) {
                    return Some(*rhs);
                }
            }
            for (x, other) in [(*lhs, *rhs), (*rhs, *lhs)] {
                if trailing
                    && matches!(b, IrBinOp::And | IrBinOp::Or)
                    && inverse_neg(other, x, ty, defs)
                {
                    return Some(x);
                }
                let mut shifted = other;
                let mut complemented = false;
                if let Some(Instruction::UnaryOp {
                    op: IrUnaryOp::Not,
                    src,
                    ty: t,
                    ..
                }) = definition(other, defs)
                {
                    if compatible(*t, ty) {
                        shifted = *src;
                        complemented = true;
                    }
                }
                if let Some(Instruction::BinOp {
                    op: s,
                    lhs: root,
                    rhs: k,
                    ty: st,
                    ..
                }) = definition(shifted, defs)
                {
                    if compatible(*st, ty)
                        && same_bits(*root, x, defs)
                        && const_in_ring(*k, ty.size() * 8)
                            .is_some_and(|k| k > 0 && k < ty.size() as u64 * 8)
                    {
                        if trailing
                            && *s == IrBinOp::Shl
                            && ((!complemented
                                && matches!(
                                    b,
                                    IrBinOp::Or | IrBinOp::Xor | IrBinOp::Add | IrBinOp::Sub
                                ))
                                || (complemented && *b == IrBinOp::And))
                        {
                            return Some(x);
                        }
                        if leading
                            && *s == IrBinOp::LShr
                            && ((!complemented && matches!(b, IrBinOp::Or | IrBinOp::Xor))
                                || (complemented && *b == IrBinOp::And))
                        {
                            return Some(x);
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}
fn count_source(
    op: IrUnaryOp,
    src: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let mut v = src;
    for _ in 0..32 {
        match count_source_once(op, v, ty, defs) {
            Some(next) if next != v => v = next,
            _ => return (v != src).then_some(v),
        }
    }
    None // Oversized/cyclic expressions fail closed.
}
fn parity(
    v: Operand,
    defs: &[Option<Instruction>],
    uses: &[u32],
    leaves: &mut Vec<(Operand, IrType)>,
    budget: &mut u8,
) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    if let Some(x) = count_input(v, defs) {
        if !private_count(v, defs, uses) {
            return false;
        }
        leaves.push(x);
        return true;
    }
    if exclusive(v, defs, uses).is_none() {
        return false;
    }
    match definition(v, defs) {
        Some(Instruction::BinOp {
            op: IrBinOp::Add | IrBinOp::Sub | IrBinOp::Xor,
            lhs,
            rhs,
            ty,
            ..
        }) if full(*ty).is_some() => {
            parity(*lhs, defs, uses, leaves, budget) && parity(*rhs, defs, uses, leaves, budget)
        }
        _ => false,
    }
}
fn normalized_parity(mut v: Operand, ty: IrType, defs: &[Option<Instruction>]) -> Operand {
    // Even-width complementation adds N mod 2 == 0 to population parity.
    let original = v;
    for _ in 0..32 {
        match definition(v, defs) {
            Some(Instruction::UnaryOp {
                op: IrUnaryOp::Not | IrUnaryOp::Bswap | IrUnaryOp::BitReverse,
                src,
                ty: t,
                ..
            }) if compatible(*t, ty) => v = *src,
            Some(Instruction::BinOp {
                op: IrBinOp::RotateLeft | IrBinOp::RotateRight,
                lhs,
                ty: t,
                ..
            }) if compatible(*t, ty) => v = *lhs,
            Some(Instruction::BinOp {
                op: IrBinOp::Xor,
                lhs,
                rhs,
                ty: t,
                ..
            }) if compatible(*t, ty) => {
                if const_in_ring(*rhs, ty.size() * 8)
                    .is_some_and(|n| n.count_ones().is_multiple_of(2))
                {
                    v = *lhs;
                } else if const_in_ring(*lhs, ty.size() * 8)
                    .is_some_and(|n| n.count_ones().is_multiple_of(2))
                {
                    v = *rhs;
                } else {
                    return v;
                }
            }
            _ => return v,
        }
    }
    original // Do not commit a partial normalization at the walk limit.
}
fn same_pair(a: Operand, b: Operand, x: Operand, y: Operand, defs: &[Option<Instruction>]) -> bool {
    (same_bits(a, x, defs) && same_bits(b, y, defs))
        || (same_bits(a, y, defs) && same_bits(b, x, defs))
}
fn cardinality(
    op: IrBinOp,
    a: Operand,
    b: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Option<Expr>, IrType)> {
    let (x, xt) = count_input(a, defs)?;
    let (y, yt) = count_input(b, defs)?;
    if !compatible(xt, yt) {
        return None;
    }
    if op == IrBinOp::Add {
        if complements(x, y, xt, defs) {
            return Some((None, xt));
        }
        if let (
            Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs: ax,
                rhs: am,
                ty: at,
                ..
            }),
            Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs: bx,
                rhs: bm,
                ty: bt,
                ..
            }),
        ) = (definition(x, defs), definition(y, defs))
        {
            if compatible(*at, xt) && compatible(*bt, xt) {
                for (root, mask) in [(*ax, *am), (*am, *ax)] {
                    for (other, nmask) in [(*bx, *bm), (*bm, *bx)] {
                        let complementary = complements(mask, nmask, xt, defs);
                        if same_bits(root, other, defs) && complementary {
                            return Some((Some(Expr::Leaf(root)), xt));
                        }
                    }
                }
            }
        }
    }
    if op == IrBinOp::Sub {
        if let (
            Some(Instruction::BinOp {
                op: IrBinOp::Or,
                lhs: ax,
                rhs: ay,
                ty: at,
                ..
            }),
            Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs: bx,
                rhs: by,
                ty: bt,
                ..
            }),
        ) = (definition(x, defs), definition(y, defs))
        {
            if compatible(*at, xt) && compatible(*bt, xt) && same_pair(*ax, *ay, *bx, *by, defs) {
                return Some((
                    Some(Expr::Bin(
                        IrBinOp::Xor,
                        Box::new(Expr::Leaf(*ax)),
                        Box::new(Expr::Leaf(*ay)),
                    )),
                    xt,
                ));
            }
        }
    }
    None
}
fn emit_count(
    dest: Value,
    expr: Expr,
    word: IrType,
    result: IrType,
    next: &mut u32,
    out: &mut Vec<Instruction>,
) {
    let src = emit(expr, word, next, out);
    if compatible(word, result) {
        out.push(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Popcount,
            src,
            ty: word,
        });
    } else {
        let tmp = Value(*next);
        *next += 1;
        out.push(Instruction::UnaryOp {
            dest: tmp,
            op: IrUnaryOp::Popcount,
            src,
            ty: word,
        });
        out.push(Instruction::Cast {
            dest,
            src: Operand::Value(tmp),
            from_ty: word,
            to_ty: result,
        });
    }
}

fn instruction_uses(i: &Instruction, mut visit: impl FnMut(&Value)) {
    crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
        if let Operand::Value(v) = o {
            visit(v);
        }
    });
    crate::backend::liveness::for_each_value_use_in_instruction(i, visit);
}

pub(super) fn run(f: &mut IrFunction) -> usize {
    // Most functions cannot contain an observation handled here. Avoid SSA
    // snapshots and operand-use tables for those functions altogether.
    if !f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::And,
                ..
            } | Instruction::UnaryOp {
                op: IrUnaryOp::Popcount
                    | IrUnaryOp::Ctz
                    | IrUnaryOp::CtzNonZero
                    | IrUnaryOp::Clz
                    | IrUnaryOp::ClzNonZero,
                ..
            }
        )
    }) {
        return 0;
    }
    let size = f.max_value_id() as usize + 1;
    let mut defs = vec![None; size];
    let mut uses = vec![0u32; size];
    for b in &f.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                // Non-SSA inputs cannot supply a stable captured value across
                // observations. Reject the whole function before making edits.
                if defs[d.0 as usize].is_some() {
                    return 0;
                }
                defs[d.0 as usize] = Some(i.clone());
            }
            instruction_uses(i, |v| uses[v.0 as usize] += 1);
        }
        crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |o| {
            if let Operand::Value(v) = o {
                uses[v.0 as usize] += 1;
            }
        });
    }
    let mut next = f.next_value_id.max(f.max_value_id().saturating_add(1));
    let mut changed = 0;
    for block in &mut f.blocks {
        let old = std::mem::take(&mut block.instructions);
        let spans = std::mem::take(&mut block.source_spans);
        for (index, i) in old.into_iter().enumerate() {
            let mut output = Vec::new();
            match &i {
                Instruction::UnaryOp { dest, op, src, ty }
                    if full(*ty).is_some() && defs[dest.0 as usize].is_some() =>
                {
                    if let Some(x) = count_source(*op, *src, *ty, &defs) {
                        output.push(Instruction::UnaryOp {
                            dest: *dest,
                            op: *op,
                            src: x,
                            ty: *ty,
                        });
                    }
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } if full(*ty).is_some() && defs[dest.0 as usize].is_some() => {
                    if matches!(op, IrBinOp::Add | IrBinOp::Sub) {
                        if let Some(plan) =
                            cardinality_synth::plan(*op, *lhs, *rhs, *ty, &defs, &uses)
                        {
                            if next.checked_add(8).is_some() {
                                match plan {
                                    cardinality_synth::Plan::Constant(n) => {
                                        output.push(Instruction::Copy {
                                            dest: *dest,
                                            src: Operand::Const(IrConst::from_i64(n, *ty)),
                                        })
                                    }
                                    cardinality_synth::Plan::Count {
                                        input,
                                        word,
                                        factor,
                                    } => {
                                        if factor == 1 {
                                            emit_count(
                                                *dest,
                                                input,
                                                word,
                                                *ty,
                                                &mut next,
                                                &mut output,
                                            );
                                        } else {
                                            let tmp = Value(next);
                                            next += 1;
                                            emit_count(
                                                tmp,
                                                input,
                                                word,
                                                *ty,
                                                &mut next,
                                                &mut output,
                                            );
                                            output.push(Instruction::BinOp {
                                                dest: *dest,
                                                op: IrBinOp::Shl,
                                                lhs: Operand::Value(tmp),
                                                rhs: Operand::Const(IrConst::from_i64(1, *ty)),
                                                ty: *ty,
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        if output.is_empty()
                            && private_count(*lhs, &defs, &uses)
                            && private_count(*rhs, &defs, &uses)
                        {
                            if let Some((expr, word)) = cardinality(*op, *lhs, *rhs, &defs) {
                                if next.checked_add(4).is_some() {
                                    if let Some(e) = expr {
                                        emit_count(*dest, e, word, *ty, &mut next, &mut output);
                                    } else {
                                        output.push(Instruction::Copy {
                                            dest: *dest,
                                            src: Operand::Const(IrConst::from_i64(
                                                (word.size() * 8) as i64,
                                                *ty,
                                            )),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if *op == IrBinOp::And {
                        let pair = const_in_ring(*rhs, ty.size() * 8)
                            .map(|m| (*lhs, m, *rhs))
                            .or_else(|| {
                                const_in_ring(*lhs, ty.size() * 8).map(|m| (*rhs, m, *lhs))
                            });
                        if let Some((value, mask, mask_op)) = pair {
                            if mask == 1 {
                                let mut leaves = Vec::new();
                                if parity(value, &defs, &uses, &mut leaves, &mut 32)
                                    && !leaves.is_empty()
                                {
                                    let word = leaves[0].1;
                                    if leaves.iter().all(|(_, t)| compatible(word, *t)) {
                                        let mut inputs = Vec::new();
                                        let mut simplified = false;
                                        for (v, t) in leaves {
                                            let root = normalized_parity(v, t, &defs);
                                            simplified |= root != v;
                                            if let Some(n) = inputs
                                                .iter()
                                                .position(|x| same_bits(*x, root, &defs))
                                            {
                                                inputs.remove(n);
                                                simplified = true;
                                            } else {
                                                inputs.push(root);
                                            }
                                        }
                                        if inputs.len() > 1 || simplified {
                                            let tree = inputs.into_iter().map(Expr::Leaf).reduce(
                                                |a, b| {
                                                    Expr::Bin(
                                                        IrBinOp::Xor,
                                                        Box::new(a),
                                                        Box::new(b),
                                                    )
                                                },
                                            );
                                            if next.checked_add(40).is_some() {
                                                if let Some(e) = tree {
                                                    let tmp = Value(next);
                                                    next += 1;
                                                    emit_count(
                                                        tmp,
                                                        e,
                                                        word,
                                                        *ty,
                                                        &mut next,
                                                        &mut output,
                                                    );
                                                    output.push(Instruction::BinOp {
                                                        dest: *dest,
                                                        op: IrBinOp::And,
                                                        lhs: Operand::Value(tmp),
                                                        rhs: mask_op,
                                                        ty: *ty,
                                                    });
                                                } else {
                                                    output.push(Instruction::Copy {
                                                        dest: *dest,
                                                        src: Operand::Const(IrConst::from_i64(
                                                            0, *ty,
                                                        )),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if output.is_empty() && mask != 0 {
                                let mut budget = 32;
                                let plan = demanded(value, mask, *ty, &defs, &uses, &mut budget);
                                if budget > 0
                                    && plan.removed > 0
                                    && next.checked_add(node_count(&plan.expr)).is_some()
                                {
                                    let word = if ty.size() == 4 {
                                        IrType::U32
                                    } else {
                                        IrType::U64
                                    };
                                    let root = emit(plan.expr, word, &mut next, &mut output);
                                    output.push(Instruction::BinOp {
                                        dest: *dest,
                                        op: IrBinOp::And,
                                        lhs: root,
                                        rhs: mask_op,
                                        ty: *ty,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => (),
            }
            if output.is_empty() {
                output.push(i);
            } else {
                changed += 1;
                // Publish each completed rewrite to the local index. A later
                // consumer must see the canonical count, not reconstruct its
                // obsolete input tree from the initial snapshot. Updating edge
                // counts also preserves the private-path cost rule after edits.
                instruction_uses(&i, |v| uses[v.0 as usize] -= 1);
                defs.resize(next as usize, None);
                uses.resize(next as usize, 0);
                for emitted in &output {
                    if let Some(d) = emitted.dest() {
                        defs[d.0 as usize] = Some(emitted.clone());
                    }
                    instruction_uses(emitted, |v| uses[v.0 as usize] += 1);
                }
            }
            if !spans.is_empty() {
                let span = spans
                    .get(index)
                    .copied()
                    .unwrap_or_else(crate::common::source::Span::dummy);
                block
                    .source_spans
                    .extend(std::iter::repeat_n(span, output.len()));
            }
            block.instructions.extend(output);
        }
    }
    f.next_value_id = next;
    changed
}
pub(super) fn early(f: &mut IrFunction) -> usize {
    if f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::URem,
                ..
            }
        )
    }) {
        run(f)
    } else {
        0
    }
}

#[cfg(test)]
#[path = "bit_idioms_demand_tests.rs"]
mod tests;

#[path = "bit_idioms_cardinality.rs"]
mod cardinality_synth;
