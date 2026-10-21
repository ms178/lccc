//! Bounded scalar reductions distilled from the scalar facts LLVM cohort.
//! All edits replace a consumer with an equivalent pure expression. Producers,
//! memory operations and shared users are untouched; no LLVM poison/no-wrap
//! flags are assumed. This runs inside the existing bit_idioms policy.
use super::*;

struct Facts(Vec<(Operand, usize, u64, u64)>);
struct Context<'a> {
    defs: &'a [Option<Instruction>],
    uses: &'a [u32],
    visits: std::cell::Cell<usize>,
}
impl Context<'_> {
    fn def(&self, op: Operand) -> Option<&Instruction> {
        let left = self.visits.get().checked_sub(1)?;
        self.visits.set(left);
        let Operand::Value(v) = op else { return None };
        self.defs.get(v.0 as usize)?.as_ref()
    }
    fn binary(&self, op: Operand, want: IrBinOp, ty: IrType) -> Option<(Operand, Operand)> {
        match self.def(op)? {
            Instruction::BinOp {
                op,
                lhs,
                rhs,
                ty: t,
                ..
            } if *op == want && *t == ty => Some((*lhs, *rhs)),
            _ => None,
        }
    }
    fn with_const(&self, op: Operand, want: IrBinOp, ty: IrType) -> Option<(Operand, u64)> {
        let (a, b) = self.binary(op, want, ty)?;
        let w = ty.size() * 8;
        const_in_ring(b, w)
            .map(|c| (a, c))
            .or_else(|| const_in_ring(a, w).map(|c| (b, c)))
    }
    fn boolean(&self, mut op: Operand) -> Option<Operand> {
        for _ in 0..16 {
            match self.def(op)? {
                Instruction::Copy { src, .. } => op = *src,
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.is_integer() && to_ty.is_integer() => op = *src,
                Instruction::Cmp { .. } => return Some(op),
                _ => return None,
            }
        }
        None
    }
    fn known(&self, op: Operand, ty: IrType, facts: &Facts, depth: u8) -> (u64, u64) {
        let w = ty.size() * 8;
        let mask = u64::MAX >> (64 - w);
        if let Some(c) = const_in_ring(op, w) {
            return ((!c & mask), c);
        }
        let mut bits = (0, 0);
        for &(x, b, z, o) in &facts.0 {
            if x == op && b == w {
                bits.0 |= z;
                bits.1 |= o;
            }
        }
        if depth == 0 {
            return bits;
        }
        let derived = match self.def(op) {
            Some(Instruction::Copy { src, .. }) => self.known(*src, ty, facts, depth - 1),
            Some(Instruction::Select {
                cond,
                true_val,
                false_val,
                ty: t,
                ..
            }) if *t == ty => match self.truth(*cond, facts, depth - 1) {
                Some(true) => self.known(*true_val, ty, facts, depth - 1),
                Some(false) => self.known(*false_val, ty, facts, depth - 1),
                None => (0, 0),
            },
            Some(Instruction::BinOp {
                op,
                lhs,
                rhs,
                ty: t,
                ..
            }) if *t == ty => {
                let a = self.known(*lhs, ty, facts, depth - 1);
                let b = self.known(*rhs, ty, facts, depth - 1);
                match op {
                    IrBinOp::And => (a.0 | b.0, a.1 & b.1),
                    IrBinOp::Or => (a.0 & b.0, a.1 | b.1),
                    IrBinOp::Xor => ((a.0 & b.0) | (a.1 & b.1), (a.0 & b.1) | (a.1 & b.0)),
                    IrBinOp::Add | IrBinOp::Sub if a.0 | a.1 == mask && b.0 | b.1 == mask => {
                        let n = if *op == IrBinOp::Add {
                            a.1.wrapping_add(b.1)
                        } else {
                            a.1.wrapping_sub(b.1)
                        } & mask;
                        (!n & mask, n)
                    }
                    _ => (0, 0),
                }
            }
            _ => (0, 0),
        };
        (bits.0 | derived.0, bits.1 | derived.1)
    }
    fn value(&self, op: Operand, ty: IrType, facts: &Facts) -> Option<u64> {
        let (z, o) = self.known(op, ty, facts, 8);
        (z | o == u64::MAX >> (64 - ty.size() * 8)).then_some(o)
    }
    fn truth(&self, op: Operand, facts: &Facts, depth: u8) -> Option<bool> {
        if depth == 0 {
            return None;
        }
        if let Operand::Const(c) = op {
            return c.to_i128().map(|n| n != 0);
        }
        match self.def(op)? {
            Instruction::Copy { src, .. } => self.truth(*src, facts, depth - 1),
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer() && to_ty.is_integer() && self.boolean(*src).is_some() => {
                self.truth(*src, facts, depth - 1)
            }
            Instruction::Cmp {
                op, lhs, rhs, ty, ..
            } if scalar(*ty) => {
                let a = self.known(*lhs, *ty, facts, 8);
                let b = self.known(*rhs, *ty, facts, 8);
                if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) && ((a.0 & b.1) | (a.1 & b.0)) != 0 {
                    return Some(*op == IrCmpOp::Ne);
                }
                let a = self.value(*lhs, *ty, facts)?;
                let b = self.value(*rhs, *ty, facts)?;
                Some(compare(*op, a, b, ty.size() * 8))
            }
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ..
            } => {
                let a = self.truth(*lhs, facts, depth - 1);
                let b = self.truth(*rhs, facts, depth - 1);
                if a == Some(false) || b == Some(false) {
                    Some(false)
                } else if a == Some(true) && b == Some(true) {
                    Some(true)
                } else {
                    None
                }
            }
            Instruction::BinOp {
                op: IrBinOp::Or,
                lhs,
                rhs,
                ..
            } => {
                let a = self.truth(*lhs, facts, depth - 1);
                let b = self.truth(*rhs, facts, depth - 1);
                if a == Some(true) || b == Some(true) {
                    Some(true)
                } else if a == Some(false) && b == Some(false) {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    fn facts(&self, op: Operand, truth: bool, out: &mut Facts, depth: u8) {
        if depth == 0 || out.0.len() >= 8 {
            return;
        }
        match self.def(op) {
            Some(Instruction::Copy { src, .. }) => self.facts(*src, truth, out, depth - 1),
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() && self.boolean(*src).is_some() => {
                self.facts(*src, truth, out, depth - 1)
            }
            Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ..
            }) if truth => {
                self.facts(*lhs, true, out, depth - 1);
                self.facts(*rhs, true, out, depth - 1);
            }
            Some(Instruction::Cmp {
                op, lhs, rhs, ty, ..
            }) if scalar(*ty) => {
                let w = ty.size() * 8;
                let mask = u64::MAX >> (64 - w);
                for (x, c) in [(*lhs, *rhs), (*rhs, *lhs)] {
                    if let Some(c) = const_in_ring(c, w) {
                        if (*op == IrCmpOp::Eq && truth) || (*op == IrCmpOp::Ne && !truth) {
                            out.0.push((x, w, !c & mask, c));
                            // Equality with an affine constant is invertible in
                            // this fixed-width ring; no no-wrap flag is needed.
                            if let Some((base, offset)) = self.with_const(x, IrBinOp::Add, *ty) {
                                let n = c.wrapping_sub(offset) & mask;
                                out.0.push((base, w, !n & mask, n));
                            } else if let Some((base, offset)) = self.binary(x, IrBinOp::Sub, *ty)
                                && let Some(offset) = const_in_ring(offset, w)
                            {
                                let n = c.wrapping_add(offset) & mask;
                                out.0.push((base, w, !n & mask, n));
                            }
                            if let Some((base, m)) = self.with_const(x, IrBinOp::And, *ty) {
                                // Impossible masked equalities supply no assumptions.
                                if c & !m == 0 {
                                    out.0.push((base, w, m & !c, c));
                                }
                            }
                        }
                    }
                }
                if matches!(op, IrCmpOp::Slt | IrCmpOp::Sge) && const_in_ring(*rhs, w) == Some(0) {
                    let sign = 1u64 << (w - 1);
                    let negative = (*op == IrCmpOp::Slt) == truth;
                    out.0.push((
                        *lhs,
                        w,
                        if negative { 0 } else { sign },
                        if negative { sign } else { 0 },
                    ));
                }
                if truth
                    && matches!(op, IrCmpOp::Ult | IrCmpOp::Ule)
                    && let Some(c) = const_in_ring(*rhs, w)
                {
                    let max = if *op == IrCmpOp::Ult {
                        c.checked_sub(1)
                    } else {
                        Some(c)
                    };
                    if let Some(max) = max {
                        let used = if max == 0 {
                            0
                        } else {
                            u64::MAX >> (max.leading_zeros())
                        };
                        out.0.push((*lhs, w, !used & mask, 0));
                    }
                }
            }
            _ => {}
        }
    }
    fn simplify_operand(&self, op: Operand, ty: IrType, facts: &Facts) -> Operand {
        // This helper substitutes conditional facts; it is not another
        // unconditional constant folder or a constant-carrier normalizer.
        if facts.0.is_empty() || matches!(op, Operand::Const(_)) {
            return op;
        }
        if let Some(n) = self.value(op, ty, facts) {
            return constant(n, ty);
        }
        if let Some(Instruction::BinOp {
            op: bin,
            lhs,
            rhs,
            ty: t,
            ..
        }) = self.def(op)
            && *t == ty
        {
            let a = self.value(*lhs, ty, facts);
            let b = self.value(*rhs, ty, facts);
            if b == Some(0)
                && matches!(
                    bin,
                    IrBinOp::Add | IrBinOp::Sub | IrBinOp::Or | IrBinOp::Xor
                )
            {
                return *lhs;
            }
            if a == Some(0) && matches!(bin, IrBinOp::Add | IrBinOp::Or | IrBinOp::Xor) {
                return *rhs;
            }
        }
        op
    }
}
fn scalar(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
    )
}
fn constant(n: u64, ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(n as i64, ty))
}
fn compare(op: IrCmpOp, a: u64, b: u64, w: usize) -> bool {
    let signed = |n: u64| ((n << (64 - w)) as i64) >> (64 - w);
    match op {
        IrCmpOp::Eq => a == b,
        IrCmpOp::Ne => a != b,
        IrCmpOp::Ult => a < b,
        IrCmpOp::Ule => a <= b,
        IrCmpOp::Ugt => a > b,
        IrCmpOp::Uge => a >= b,
        IrCmpOp::Slt => signed(a) < signed(b),
        IrCmpOp::Sle => signed(a) <= signed(b),
        IrCmpOp::Sgt => signed(a) > signed(b),
        IrCmpOp::Sge => signed(a) >= signed(b),
    }
}
fn bin(dest: Value, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    }
}
fn copy(dest: Value, src: Operand) -> Vec<Instruction> {
    vec![Instruction::Copy { dest, src }]
}

fn fold(
    inst: &Instruction,
    cx: &Context<'_>,
    facts: &Facts,
    fresh: Value,
) -> Option<Vec<Instruction>> {
    cx.visits.set(512);
    if let Some(replacement) = count_merge::fold(inst, cx, facts, fresh) {
        return Some(replacement);
    }
    if let Some(replacement) = guards::fold(inst, cx, facts, fresh) {
        return Some(replacement);
    }
    let dest = inst.dest()?;
    if let Instruction::Select {
        cond,
        true_val,
        false_val,
        ..
    } = inst
        && boolean_tree(*true_val, cx, 8)
        && boolean_tree(*false_val, cx, 8)
    {
        let mut local = Facts(facts.0.clone());
        cx.facts(*cond, true, &mut local, 8);
        let a = cx.truth(*true_val, &local, 8);
        let b = cx.truth(*false_val, &local, 8);
        if a.is_some() && a == b {
            return Some(copy(dest, *false_val));
        }
    }
    // The non-Boolean case is specifically a nested constant-choice
    // observation, not a general select-DAG simplifier. In particular, leave
    // count-idiom accumulator networks intact for the existing recognizer.
    let constant_choice = |op| {
        matches!(
            cx.def(op),
            Some(Instruction::Select {
                true_val: Operand::Const(_),
                false_val: Operand::Const(_),
                ..
            })
        )
    };
    if let Instruction::Select {
        cond,
        true_val,
        false_val,
        ty,
        ..
    } = inst
        && scalar(*ty)
        && ((constant_choice(*true_val) && matches!(false_val, Operand::Const(_)))
            || (constant_choice(*false_val) && matches!(true_val, Operand::Const(_))))
    {
        let mut yes_facts = Facts(facts.0.clone());
        let mut no_facts = Facts(facts.0.clone());
        cx.facts(*cond, true, &mut yes_facts, 8);
        cx.facts(*cond, false, &mut no_facts, 8);
        let yes = cx.simplify_operand(*true_val, *ty, &yes_facts);
        let no = cx.simplify_operand(*false_val, *ty, &no_facts);
        if yes != *true_val || no != *false_val {
            if const_in_ring(yes, ty.size() * 8) == Some(1)
                && const_in_ring(no, ty.size() * 8) == Some(0)
                && boolean_tree(*cond, cx, 8)
            {
                return Some(copy(dest, *cond));
            }
            return Some(vec![Instruction::Select {
                dest,
                cond: *cond,
                true_val: yes,
                false_val: no,
                ty: *ty,
            }]);
        }
    }
    if let Instruction::BinOp {
        op: IrBinOp::And,
        lhs,
        rhs,
        ty,
        ..
    } = inst
    {
        // Boolean implication/substitution; neither operand may be an arbitrary
        // integer whose nonzero value differs from canonical true.
        for (guard, other) in [(*lhs, *rhs), (*rhs, *lhs)] {
            if !boolean_tree(guard, cx, 8) || !boolean_tree(other, cx, 8) {
                continue;
            }
            let mut local = Facts(facts.0.clone());
            cx.facts(guard, true, &mut local, 8);
            match cx.truth(other, &local, 8) {
                Some(true) => return Some(copy(dest, guard)),
                Some(false) => return Some(copy(dest, constant(0, *ty))),
                None => {}
            }
            if let Some(Instruction::Cmp {
                op,
                lhs: a,
                rhs: b,
                ty: t,
                ..
            }) = cx.def(other)
                && scalar(*t)
            {
                let x = cx.simplify_operand(*a, *t, &local);
                let y = cx.simplify_operand(*b, *t, &local);
                if x != *a || y != *b {
                    return Some(vec![
                        Instruction::Cmp {
                            dest: fresh,
                            op: *op,
                            lhs: x,
                            rhs: y,
                            ty: *t,
                        },
                        bin(dest, IrBinOp::And, guard, Operand::Value(fresh), *ty),
                    ]);
                }
            }
            // Exact small unsigned modular range, not a signed-range guess.
            if let Some(Instruction::Cmp {
                op: IrCmpOp::Ult,
                lhs: a,
                rhs: b,
                ty: t,
                ..
            }) = cx.def(guard)
                && scalar(*t)
                && let Some(count) = const_in_ring(*b, t.size() * 8)
                && (1..=16).contains(&count)
                && let Some((x, offset)) = cx.with_const(*a, IrBinOp::Add, *t)
            {
                let w = t.size() * 8;
                let mask = u64::MAX >> (64 - w);
                let mut hits = Vec::new();
                let mut known = true;
                for i in 0..count {
                    let n = i.wrapping_sub(offset) & mask;
                    let mut f = Facts(facts.0.clone());
                    f.0.push((x, w, !n & mask, n));
                    match cx.truth(other, &f, 8) {
                        Some(true) => hits.push(n),
                        Some(false) => {}
                        None => {
                            known = false;
                            break;
                        }
                    }
                }
                if known && hits.len() == 1 {
                    return Some(vec![Instruction::Cmp {
                        dest,
                        op: IrCmpOp::Eq,
                        lhs: x,
                        rhs: constant(hits[0], *t),
                        ty: *t,
                    }]);
                }
            }
        }
    }
    if let Instruction::Cmp {
        op, lhs, rhs, ty, ..
    } = inst
        && scalar(*ty)
    {
        if !facts.0.is_empty()
            && let Some(b) = cx.truth(Operand::Value(dest), facts, 8)
        {
            return Some(copy(dest, constant(u64::from(b), IrType::I32)));
        }
        if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            let w = ty.size() * 8;
            for (value, k) in [(*lhs, *rhs), (*rhs, *lhs)] {
                let Some(k) = const_in_ring(k, w) else {
                    continue;
                };
                if k <= 1
                    && let Some((a, b)) = cx.binary(value, IrBinOp::And, *ty)
                {
                    for (x, neg) in [(a, b), (b, a)] {
                        let is_neg = matches!(cx.def(neg),Some(Instruction::UnaryOp{op:IrUnaryOp::Neg,src,ty:t,..}) if *src==x&&*t==*ty)
                            || matches!(cx.def(neg),Some(Instruction::BinOp{op:IrBinOp::Sub,lhs:a,rhs:b,ty:t,..}) if *b==x&&*t==*ty&&const_in_ring(*a,w)==Some(0));
                        if is_neg {
                            if k == 0 {
                                return Some(vec![Instruction::Cmp {
                                    dest,
                                    op: *op,
                                    lhs: x,
                                    rhs: constant(0, *ty),
                                    ty: *ty,
                                }]);
                            }
                            return Some(vec![
                                bin(fresh, IrBinOp::And, x, constant(1, *ty), *ty),
                                Instruction::Cmp {
                                    dest,
                                    op: *op,
                                    lhs: Operand::Value(fresh),
                                    rhs: constant(1, *ty),
                                    ty: *ty,
                                },
                            ]);
                        }
                    }
                }
                if k == 0
                    && let Some((a, b)) = cx.binary(value, IrBinOp::Or, *ty)
                {
                    for (x, boolean) in [(a, b), (b, a)] {
                        if let Some(Instruction::Cmp {
                            op: p,
                            lhs: u,
                            rhs: v,
                            ty: t,
                            ..
                        }) = cx.boolean(boolean).and_then(|b| cx.def(b))
                            && t.size() == ty.size()
                            && matches!(p, IrCmpOp::Ne | IrCmpOp::Ugt)
                            && ((*u == x && const_in_ring(*v, w) == Some(0))
                                || (*p == IrCmpOp::Ne
                                    && *v == x
                                    && const_in_ring(*u, w) == Some(0)))
                        {
                            return Some(vec![Instruction::Cmp {
                                dest,
                                op: *op,
                                lhs: x,
                                rhs: constant(0, *ty),
                                ty: *ty,
                            }]);
                        }
                    }
                }
            }
        }
    }
    arithmetic(inst, cx, fresh)
}
fn boolean_tree(op: Operand, cx: &Context<'_>, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }
    if cx.boolean(op).is_some() {
        return true;
    }
    match cx.def(op) {
        Some(Instruction::BinOp {
            op: IrBinOp::And | IrBinOp::Or,
            lhs,
            rhs,
            ..
        }) => boolean_tree(*lhs, cx, depth - 1) && boolean_tree(*rhs, cx, depth - 1),
        _ => matches!(op,Operand::Const(c) if matches!(c.to_i128(),Some(0|1))),
    }
}

fn scale(op: Operand, ty: IrType, cx: &Context<'_>) -> Option<(Operand, u64)> {
    // Earlier canonicalization spells multiplication by two as x+x.
    if let Some((a, b)) = cx.binary(op, IrBinOp::Add, ty)
        && a == b
    {
        return Some((a, 2));
    }
    if let Some(x) = cx.with_const(op, IrBinOp::Mul, ty) {
        return Some(x);
    }
    let (x, s) = cx.binary(op, IrBinOp::Shl, ty)?;
    let s = const_in_ring(s, ty.size() * 8)?;
    (s < ty.size() as u64 * 8).then(|| (x, 1u64 << s))
}
fn division(op: Operand, ty: IrType, cx: &Context<'_>) -> Option<(Operand, u64)> {
    if let Some((x, d)) = cx.binary(op, IrBinOp::UDiv, ty) {
        return Some((x, const_in_ring(d, ty.size() * 8)?));
    }
    let (x, s) = cx.binary(op, IrBinOp::LShr, ty)?;
    let s = const_in_ring(s, ty.size() * 8)?;
    (s < ty.size() as u64 * 8).then(|| (x, 1u64 << s))
}
fn remainder(op: Operand, ty: IrType, cx: &Context<'_>) -> Option<(Operand, u64)> {
    if let Some((x, d)) = cx.binary(op, IrBinOp::URem, ty) {
        return Some((x, const_in_ring(d, ty.size() * 8)?));
    }
    let (x, m) = cx.with_const(op, IrBinOp::And, ty)?;
    let d = m.checked_add(1)?;
    d.is_power_of_two().then_some((x, d))
}
fn arithmetic(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    let Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !matches!(ty, IrType::U32 | IrType::U64) {
        return None;
    }
    let w = ty.size() * 8;
    let ring = u64::MAX >> (64 - w);
    if *op == IrBinOp::And {
        for (value, m) in [(*lhs, *rhs), (*rhs, *lhs)] {
            let Some(m) = const_in_ring(m, w) else {
                continue;
            };
            if m == 0 || m == ring || !(m + 1).is_power_of_two() {
                continue;
            }
            let Some((shifted, b)) = cx.with_const(value, IrBinOp::Add, *ty) else {
                continue;
            };
            let Some((added, s)) = cx.binary(shifted, IrBinOp::LShr, *ty) else {
                continue;
            };
            let Some(s) = const_in_ring(s, w) else {
                continue;
            };
            if s == 0 || s >= w as u64 || m > (ring >> s) {
                continue;
            }
            let Some((x, a)) = cx.with_const(added, IrBinOp::Add, *ty) else {
                continue;
            };
            if a & ((1u64 << s) - 1) == 0 && ((a >> s).wrapping_add(b) & m) == 0 {
                return Some(vec![
                    bin(fresh, IrBinOp::LShr, x, constant(s, *ty), *ty),
                    bin(
                        *dest,
                        IrBinOp::And,
                        Operand::Value(fresh),
                        constant(m, *ty),
                        *ty,
                    ),
                ]);
            }
        }
    }
    if *op != IrBinOp::Add {
        return None;
    }
    for (qa, ra) in [(*lhs, *rhs), (*rhs, *lhs)] {
        let Some((q, qs)) = scale(qa, *ty, cx) else {
            continue;
        };
        let Some((x, d)) = division(q, *ty, cx) else {
            continue;
        };
        if d == 0 {
            continue;
        }
        let (rterm, mask) = cx
            .with_const(ra, IrBinOp::And, *ty)
            .map_or((ra, None), |(v, m)| (v, Some(m)));
        let Some((r, s)) = scale(rterm, *ty, cx) else {
            continue;
        };
        let Some((y, rd)) = remainder(r, *ty, cx) else {
            continue;
        };
        if x != y || d != rd || s == 0 || d.wrapping_mul(s) & ring != qs {
            continue;
        }
        let one_use =
            |v: Operand| matches!(v,Operand::Value(v) if cx.uses.get(v.0 as usize)==Some(&1));
        if !one_use(qa) && !one_use(ra) {
            continue;
        }
        let product = bin(*dest, IrBinOp::Mul, x, constant(s, *ty), *ty);
        let Some(m) = mask else {
            return Some(vec![product]);
        };
        if m == 0 {
            continue;
        }
        let k = m.trailing_zeros();
        let high = ring & (u64::MAX << k);
        let max = (d as u128 - 1) * s as u128;
        if max > ring as u128 || qs.trailing_zeros() < k {
            continue;
        }
        let used = if max == 0 {
            0
        } else {
            u64::MAX >> (max as u64).leading_zeros()
        };
        if (m ^ high) & used != 0 {
            continue;
        }
        return Some(vec![
            bin(fresh, IrBinOp::Mul, x, constant(s, *ty), *ty),
            bin(
                *dest,
                IrBinOp::And,
                Operand::Value(fresh),
                constant(high, *ty),
                *ty,
            ),
        ]);
    }
    None
}

pub(super) fn run(func: &mut IrFunction) -> usize {
    use crate::ir::reexports::Terminator;
    if func
        .blocks
        .iter()
        .any(|b| !b.source_spans.is_empty() && b.source_spans.len() != b.instructions.len())
    {
        return 0;
    }
    // max_value_id includes the next-ID watermark. Bound the dense snapshot
    // BEFORE allocating: exhausted/sparse IDs must lose this optimization,
    // not attempt a terabyte allocation while checking for ID exhaustion.
    let max_id = func.max_value_id();
    if func.next_value_id == u32::MAX || max_id > 1_000_000 {
        return 0;
    }
    let mut defs = vec![None; max_id as usize + 1];
    let mut uses = vec![0u32; defs.len()];
    for b in &func.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                if defs[d.0 as usize].is_some() {
                    return 0;
                }
                defs[d.0 as usize] = Some(i.clone());
            }
            crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
                if let Operand::Value(v) = o {
                    uses[v.0 as usize] = uses[v.0 as usize].saturating_add(1)
                }
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |o| {
            if let Operand::Value(v) = o {
                uses[v.0 as usize] = uses[v.0 as usize].saturating_add(1)
            }
        });
    }
    let cx = Context {
        defs: &defs,
        uses: &uses,
        visits: std::cell::Cell::new(512),
    };
    let mut next = func
        .next_value_id
        .max(func.max_value_id().saturating_add(1));
    let mut plans = Vec::new();
    let labels = crate::ir::analysis::build_label_map(func);
    if labels.len() != func.blocks.len() {
        return 0;
    }
    let (preds, _) = crate::ir::analysis::build_cfg(func, &labels);
    for (bi, b) in func.blocks.iter().enumerate() {
        cx.visits.set(512);
        let mut facts = Facts(vec![]);
        // A unique incoming edge is a dominance certificate for its condition.
        // Do not assume a fact merely because one of several predecessors has it.
        if bi != 0 && preds.len(bi) == 1 {
            let pred = &func.blocks[preds.row(bi)[0] as usize];
            if let Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } = &pred.terminator
                && true_label != false_label
            {
                cx.facts(*cond, *true_label == b.label, &mut facts, 8);
            }
        }
        for (ii, i) in b.instructions.iter().enumerate() {
            let Some(after) = next.checked_add(1) else {
                return 0;
            };
            if let Some(replacement) = fold(i, &cx, &facts, Value(next)) {
                if replacement.len() > 1 {
                    next = after;
                }
                plans.push((bi, ii, replacement));
            }
        }
    }
    let changes = plans.len();
    for (bi, ii, replacement) in plans.into_iter().rev() {
        let b = &mut func.blocks[bi];
        if !b.source_spans.is_empty() {
            let span = b.source_spans[ii];
            b.source_spans
                .splice(ii..=ii, std::iter::repeat_n(span, replacement.len()));
        }
        b.instructions.splice(ii..=ii, replacement);
    }
    func.next_value_id = next;
    changes + guards::cfg(func)
}

#[cfg(test)]
#[path = "bit_idioms_facts_tests.rs"]
mod tests;

#[path = "bit_idioms_guards.rs"]
mod guards;

#[path = "bit_idioms_count_merge.rs"]
mod count_merge;
