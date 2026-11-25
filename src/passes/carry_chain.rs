//! Full adders: `x + y + carry` with its carry out, for `adc` and `sbb`.
//!
//! Multi-word arithmetic adds limb by limb, each addition taking the carry
//! of the one before. C has no carry, so every source spells it out, and
//! they all spell it the same way:
//!
//! ```text
//!     s1 = x + y            d1 = x - y
//!     s2 = s1 + c           d2 = d1 - c
//!     c1 = s1 <u x          b1 = x <u y
//!     c2 = s2 <u s1         b2 = d1 <u c
//!     out = c1 | c2         out = b1 | b2
//! ```
//!
//! `_addcarry_u64` and `_subborrow_u64` of `x86gprintrin.h` are this,
//! `__builtin_addcll` and `__builtin_subcll` are lowered to this, and
//! portable limb arithmetic is written like this. Lowered one instruction at
//! a time it is fourteen instructions a limb; the machine has one, `adc`.
//!
//! The variations are few, and all of them are matched: either addend in
//! a comparison (`s1 <u y`, `s2 <u c`); a borrow that says the difference
//! grew (`d1 >u x`, `d2 >u d1`); the carry added first (`(x + c) + y`,
//! mini-gmp and OpenSSL); the subtrahends added up first (`x - (y + c)`
//! with `(y + c) <u c` and `x <u (y + c)`, mini-gmp); and the two partial
//! carries joined by `+`, `^` or `||` instead of `|` (`cy = s < a; ...;
//! cy += t < s`, and what `(unsigned __int128)a + b + c` is split into).
//!
//! The pass rewrites the five instructions into the two halves of
//! [`IntrinsicOp::Carry`], adjacent and on the same operands:
//!
//! ```text
//!     s2  = Carry{word}(x, y, c)      ; in place of `s2 = s1 + c`
//!     out = Carry{out}(x, y, c)       ; directly behind it
//! ```
//!
//! which the x86-64 backend emits as one `adc` and one `setc`
//! (`emit_carry_impl`); the `setc`/`bt` pair between two links of a chain is
//! removed by the peephole (`multiword::forward_carry_flag`).
//!
//! A chain starts with an addition that has no carry in. Its carry out is
//! the comparison `s1 <u x`, which costs a `cmp` the addition has already
//! made; where that comparison feeds nothing but the next link, the
//! addition becomes the pair with a constant 0 carry ([`chain_heads`]): an
//! `add` and its `setc`. And it ends with an addition whose carry out
//! nobody reads, `(x + y) + c`: the word half alone ([`chain_tails`]).
//!
//! # Why an IR rewrite and not a fused emission
//!
//! The five instructions are not adjacent: the store of `s2` sits between
//! the second addition and the comparisons, and `y` is dead by then. A
//! backend that emitted the group at one of its members would define `out`
//! before, or `s2` after, the point the register allocator placed them at.
//! Rewritten here, ahead of the allocator, the pair is two ordinary
//! instructions with ordinary live ranges.
//!
//! # Soundness
//!
//! * `c` is proven 0 or 1 ([`booleans`]): a comparison, a carry out, a
//!   constant 0 or 1, a conversion or a copy of one, `v & 1`, a bitwise
//!   combination of two, and a phi or select of nothing else. For such `c`,
//!   `s2 = x + y + c` and `out` is its carry: at most one of the two partial
//!   carries is set, so their `|`, their `+` and their `^` are the same
//!   bit. (A carry in that is not proven a bit is left alone:
//!   `__builtin_addcll(a, b, n, &o)` adds any `n`, and `adc` adds one bit;
//!   and of three words the two carries may both be set, which `+` counts
//!   and `^` cancels.) `c` is the bit itself, found behind the conversions
//!   and the `!= 0` the sources write.
//! * Which operand is the bit does not matter to an addition, and of
//!   `x - y - w` it may be `y` or `w` but not `x` ([`Shape::bits`]).
//! * The `+` of an adder's partial carries is itself a bit where one of
//!   the adder's operands is, which round a loop is that same `+`: the
//!   greatest fixed point of [`booleans`] holds by induction over the
//!   trips, and falls as soon as the loop is entered with a word.
//! * The new instructions read `x`, `y` and `c`, which the old ones read
//!   (transitively) at the same place or earlier, so every definition still
//!   dominates its uses. `out` moves up to behind the word, which dominated
//!   the `|` -- by construction where the `|` reads a comparison of the word
//!   (every addition, and the subtraction spelled `d2 >u d1`). Where it
//!   reads none (`d1 <u c`, and `x <u (y + c)`), the word is looked for in
//!   the block of the `|` ([`word_near`]): one ahead of the `|` takes the
//!   pair, and one behind it is computed where the `|` was, under a new
//!   name, so that either way both halves stand ahead of every reader of
//!   the word and of the `|`. The word is arithmetic on values and reads no
//!   memory, so where in the block it is computed changes nothing else.
//! * A carry out is 0 or 1 in a whole register, as a comparison's result
//!   is, and takes the place of the `|` wherever that is read at 32 bits or
//!   fewer; a 64-bit `|` becomes a conversion of it.
//! * Nothing with an effect is removed or duplicated: the old comparisons,
//!   the `|` and the first addition are left for dead-code elimination, and
//!   survive where something else reads them.
//!
//! x86-64 only (the one backend that emits the pair); `CCC_DISABLE_PASSES=
//! carry` turns it off.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::intrinsics::CarryOp;
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value,
};

/// Block and instruction index.
type Position = (usize, usize);

/// What the matcher asks about the function: where a value is defined, how
/// often it is read, and (computed when the first candidate needs it) which
/// values are bits.
struct Facts {
    defs: Vec<Option<Position>>,
    uses: Vec<u32>,
    bools: Option<FxHashSet<u32>>,
}

impl Facts {
    fn of(func: &IrFunction) -> Self {
        let bound = func.sound_next_value_id() as usize;
        let mut defs = vec![None; bound];
        let mut uses = vec![0u32; bound];
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(Value(v)) = inst.dest() {
                    defs[v as usize] = Some((bi, ii));
                }
                inst.for_each_used_value(|v| uses[v as usize] += 1);
            }
            block
                .terminator
                .for_each_used_value(|v| uses[v as usize] += 1);
        }
        Self {
            defs,
            uses,
            bools: None,
        }
    }

    fn def<'a>(&self, func: &'a IrFunction, v: Value) -> Option<&'a Instruction> {
        let (bi, ii) = (*self.defs.get(v.0 as usize)?)?;
        Some(&func.blocks[bi].instructions[ii])
    }

    fn read_once(&self, v: Value) -> bool {
        self.uses.get(v.0 as usize) == Some(&1)
    }

    fn is_bool(&mut self, func: &IrFunction, v: Value) -> bool {
        if self.bools.is_none() {
            self.bools = Some(booleans(func, self));
        }
        self.bools.as_ref().is_some_and(|bits| bits.contains(&v.0))
    }
}

fn const_bit(operand: &Operand) -> Option<bool> {
    match operand {
        Operand::Const(c) => match c.to_i64() {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        },
        Operand::Value(_) => None,
    }
}

fn is_integer(ty: IrType) -> bool {
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

/// The values that are 0 or 1 whatever the program does.
///
/// The greatest fixed point: every candidate is taken for a bit, and one
/// that reads something which is not a bit is struck, with everything that
/// reads it, so a carry that travels round a loop through a phi (`c = 0;
/// for (...) c = adc(c, ...)`) is found. Linear in the candidates and their
/// operands.
///
/// A `+` of two bits is not a bit, except the one sources write most: the
/// sum of an adder's two partial carries (`c = s < a; ...; c += t < s`,
/// and the high half of `(unsigned __int128)a + b + c`). Those two are
/// never both set where one of the adder's operands is a bit
/// ([`Shape::bits`]), so the sum is a candidate that stands for as long as
/// one of them does -- which, round a loop, is the sum itself.
fn booleans(func: &IrFunction, facts: &Facts) -> FxHashSet<u32> {
    // What each candidate reads, and whether it reads a constant other
    // than 0 or 1.
    let mut reads: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut struck: Vec<u32> = Vec::new();
    // The sums of partial carries, with the operands of which one has to
    // be a bit.
    let mut sums: Vec<(u32, Vec<Operand>)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let (dest, operands): (Value, Vec<&Operand>) = match inst {
                Instruction::Cmp { dest, .. }
                | Instruction::Intrinsic {
                    dest: Some(dest),
                    op: IntrinsicOp::Carry(CarryOp { out: true, .. }),
                    ..
                } => (*dest, Vec::new()),
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } if is_integer(*from_ty) && is_integer(*to_ty) => (*dest, vec![src]),
                Instruction::Copy { dest, src } => (*dest, vec![src]),
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::And,
                    lhs,
                    rhs,
                    ty,
                } if is_integer(*ty) => {
                    // `v & 1` is a bit whatever `v` is.
                    if const_bit(lhs).is_some() || const_bit(rhs).is_some() {
                        (*dest, Vec::new())
                    } else {
                        (*dest, vec![lhs, rhs])
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Or | IrBinOp::Xor,
                    lhs,
                    rhs,
                    ty,
                } if is_integer(*ty) => (*dest, vec![lhs, rhs]),
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty,
                } if is_integer(*ty) => {
                    let Some(shape) = shape_at(func, facts, (bi, ii)) else {
                        continue;
                    };
                    sums.push((dest.0, shape.bits().into_iter().cloned().collect()));
                    // The partial carries themselves: comparisons.
                    (*dest, vec![lhs, rhs])
                }
                Instruction::Select {
                    dest,
                    true_val,
                    false_val,
                    ty,
                    ..
                } if is_integer(*ty) => (*dest, vec![true_val, false_val]),
                Instruction::Phi { dest, incoming, ty } if is_integer(*ty) => {
                    (*dest, incoming.iter().map(|(op, _)| op).collect())
                }
                _ => continue,
            };
            let mut values = Vec::with_capacity(operands.len());
            let mut constant_word = false;
            for operand in operands {
                match operand {
                    Operand::Value(v) => values.push(v.0),
                    Operand::Const(_) => constant_word |= const_bit(operand).is_none(),
                }
            }
            if constant_word {
                struck.push(dest.0);
            }
            reads.insert(dest.0, values);
        }
    }
    // Who reads whom, among the candidates; a read of a value that is not a
    // candidate strikes the reader at once.
    let mut readers: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (&dest, values) in &reads {
        for &v in values {
            if reads.contains_key(&v) {
                readers.entry(v).or_default().push(dest);
            } else {
                struck.push(dest);
            }
        }
    }
    // A sum stands while one of its adder's operands may be a bit: how
    // many of them still may, and which sums each of them holds up. A
    // constant bit holds its sum up for good.
    let mut standing: FxHashMap<u32, u32> = FxHashMap::default();
    let mut holds_up: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (sum, operands) in &sums {
        let mut count = 0u32;
        for operand in operands {
            match operand {
                Operand::Const(_) if const_bit(operand).is_some() => count += 1,
                Operand::Value(v) if reads.contains_key(&v.0) => {
                    count += 1;
                    holds_up.entry(v.0).or_default().push(*sum);
                }
                _ => {}
            }
        }
        if count == 0 {
            struck.push(*sum);
        }
        standing.insert(*sum, count);
    }
    let mut set: FxHashSet<u32> = reads.keys().copied().collect();
    while let Some(v) = struck.pop() {
        if !set.remove(&v) {
            continue;
        }
        if let Some(dependents) = readers.get(&v) {
            struck.extend(dependents.iter().copied().filter(|d| set.contains(d)));
        }
        for sum in holds_up.get(&v).into_iter().flatten() {
            if let Some(count) = standing.get_mut(sum) {
                *count -= 1;
                if *count == 0 {
                    struck.push(*sum);
                }
            }
        }
    }
    set
}

/// The carry behind `operand`: the bit the source converted (`(unsigned
/// long long)(c != 0)`), or a constant bit. `None` when the operand is not
/// proven 0 or 1.
fn carry_of(func: &IrFunction, facts: &mut Facts, operand: &Operand) -> Option<Operand> {
    if let Some(bit) = const_bit(operand) {
        return Some(Operand::Const(IrConst::I8(bit as i8)));
    }
    let &Operand::Value(mut v) = operand else {
        return None;
    };
    if !facts.is_bool(func, v) {
        return None;
    }
    // Towards the value that was converted, for as long as the next one is
    // a bit too.
    for _ in 0..16 {
        let inner = match facts.def(func, v) {
            Some(Instruction::Cast { src, .. } | Instruction::Copy { src, .. }) => src,
            Some(Instruction::Cmp {
                op: IrCmpOp::Ne,
                lhs,
                rhs,
                ..
            }) if const_bit(rhs) == Some(false) => lhs,
            _ => break,
        };
        match inner {
            Operand::Const(_) => {
                return const_bit(inner).map(|bit| Operand::Const(IrConst::I8(bit as i8)));
            }
            Operand::Value(next) if facts.is_bool(func, *next) => v = *next,
            Operand::Value(_) => break,
        }
    }
    Some(Operand::Value(v))
}

/// `value` is `a <u b`, in either spelling: `(a, b, type compared at)`.
fn unsigned_less(
    func: &IrFunction,
    facts: &Facts,
    value: Value,
) -> Option<(Operand, Operand, IrType)> {
    match facts.def(func, value)? {
        Instruction::Cmp {
            op: IrCmpOp::Ult,
            lhs,
            rhs,
            ty,
            ..
        } => Some((lhs.clone(), rhs.clone(), *ty)),
        Instruction::Cmp {
            op: IrCmpOp::Ugt,
            lhs,
            rhs,
            ty,
            ..
        } => Some((rhs.clone(), lhs.clone(), *ty)),
        _ => None,
    }
}

/// `operand` is the result of a `wanted` operation: `(it, lhs, rhs, type)`.
fn binop(
    func: &IrFunction,
    facts: &Facts,
    operand: &Operand,
    wanted: IrBinOp,
) -> Option<(Value, Operand, Operand, IrType)> {
    let Operand::Value(v) = operand else {
        return None;
    };
    match facts.def(func, *v)? {
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } if *op == wanted => Some((*v, lhs.clone(), rhs.clone(), *ty)),
        _ => None,
    }
}

/// The adder of unsigned words of type `ty`, if the machine has one.
fn word_op(ty: IrType, borrow: bool) -> Option<CarryOp> {
    let wide = match ty {
        IrType::U64 => true,
        IrType::U32 => false,
        _ => return None,
    };
    Some(CarryOp {
        borrow,
        out: false,
        wide,
    })
}

/// The comparison behind an operand of the `|`: the operand itself, or what
/// a conversion of it converts. Each link is read once, so that all of it
/// dies with the `|`.
fn partial_carry(func: &IrFunction, facts: &Facts, mut v: Value) -> Option<Value> {
    for _ in 0..4 {
        if !facts.read_once(v) {
            return None;
        }
        match facts.def(func, v)? {
            Instruction::Cmp { .. } => return Some(v),
            Instruction::Cast {
                src: Operand::Value(src),
                from_ty,
                to_ty,
                ..
            } if is_integer(*from_ty) && is_integer(*to_ty) => v = *src,
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => v = *src,
            _ => return None,
        }
    }
    None
}

/// The two operations whose partial carries a join combines, before any
/// of their operands is known to be a bit: `(x + y) + w` or `(x - y) - w`.
struct Shape {
    /// The second operation, `s2 = s1 + w` (or `d2 = d1 - w`): where the
    /// word half goes.
    word_at: Position,
    word: Value,
    op: CarryOp,
    x: Operand,
    y: Operand,
    w: Operand,
}

impl Shape {
    /// The operands of which one has to be 0 or 1 for the two partial
    /// carries never to be both set, the one the sources mean first.
    ///
    /// Of `x + y + w`, any: the sum of two words and a bit is less than
    /// twice 2^n, so it carries once at most. Of `x - y - w`, either
    /// subtrahend: a word and a bit are 2^n together at most, so the
    /// difference borrows once at most. (Not `x`: `1 - y - w` borrows
    /// twice for `y = w = 2^n - 1`.)
    fn bits(&self) -> Vec<&Operand> {
        if self.op.borrow {
            vec![&self.w, &self.y]
        } else {
            vec![&self.w, &self.x, &self.y]
        }
    }

    /// The operands of the halves, `(x, y, c)`, with `c` the bit behind
    /// the first operand that is proven one.
    fn args(&self, func: &IrFunction, facts: &mut Facts) -> Option<[Operand; 3]> {
        let (at, c) = self
            .bits()
            .into_iter()
            .enumerate()
            .find_map(|(at, operand)| Some((at, carry_of(func, facts, operand)?)))?;
        let (x, y, w) = (self.x.clone(), self.y.clone(), self.w.clone());
        Some(match (self.op.borrow, at) {
            // `w` is the carry.
            (_, 0) => [x, y, c],
            // The subtraction's `y`, or the addition's `x`.
            (true, _) => [x, w, c],
            (false, 1) => [y, w, c],
            (false, _) => [x, w, c],
        })
    }
}

/// `c1 = s1 <u x|y` with `s1 = x + y`; `c2 = s2 <u s1|w` with `s2 = s1 + w`.
fn add_shape(func: &IrFunction, facts: &Facts, c1: Value, c2: Value) -> Option<Shape> {
    let (s1_operand, bound1, ty1) = unsigned_less(func, facts, c1)?;
    let (s1, x, y, add1_ty) = binop(func, facts, &s1_operand, IrBinOp::Add)?;
    if bound1 != x && bound1 != y {
        return None;
    }
    let (s2_operand, bound2, ty2) = unsigned_less(func, facts, c2)?;
    let (s2, a, b, add2_ty) = binop(func, facts, &s2_operand, IrBinOp::Add)?;
    let s1_operand = Operand::Value(s1);
    let w = if a == s1_operand {
        b
    } else if b == s1_operand {
        a
    } else {
        return None;
    };
    if bound2 != s1_operand && bound2 != w {
        return None;
    }
    let op = word_op(add1_ty, false)?;
    if [add2_ty, ty1, ty2]
        .iter()
        .any(|ty| word_op(*ty, false) != Some(op))
    {
        return None;
    }
    let word_at = (*facts.defs.get(s2.0 as usize)?)?;
    Some(Shape {
        word_at,
        word: s2,
        op,
        x,
        y,
        w,
    })
}

/// A word that no comparison reads (`d1 <u w` tests the operands of
/// `d2 = d1 - w`, not `d2`) is looked for in the block of the join: the
/// nearest one ahead of it, or else the first one behind it.
fn word_near(
    func: &IrFunction,
    join_at: Position,
    is_word: impl Fn(&Instruction) -> bool,
) -> Option<(Position, Value)> {
    let (bi, join_index) = join_at;
    let insts = &func.blocks[bi].instructions;
    let found = |(ii, inst): (usize, &Instruction)| {
        is_word(inst)
            .then(|| inst.dest())
            .flatten()
            .map(|word| ((bi, ii), word))
    };
    insts[..join_index]
        .iter()
        .enumerate()
        .rev()
        .find_map(found)
        .or_else(|| {
            insts
                .iter()
                .enumerate()
                .skip(join_index + 1)
                .find_map(found)
        })
}

/// `b1 = x <u y` (or `x <u d1`: the difference is the greater) with
/// `d1 = x - y`; `b2 = d1 <u w` (or `d1 <u d2`) with `d2 = d1 - w`.
/// `join_at` is where the join is ([`word_near`]).
fn sub_shape(
    func: &IrFunction,
    facts: &Facts,
    b1: Value,
    b2: Value,
    join_at: Position,
) -> Option<Shape> {
    let (first, second, ty1) = unsigned_less(func, facts, b1)?;
    let (d1_operand, bound, ty2) = unsigned_less(func, facts, b2)?;
    let (d1, x, y, sub1_ty) = binop(func, facts, &d1_operand, IrBinOp::Sub)?;
    let d1_operand = Operand::Value(d1);
    if first != x || (second != y && second != d1_operand) {
        return None;
    }
    let op = word_op(sub1_ty, true)?;
    if word_op(ty1, true) != Some(op) || word_op(ty2, true) != Some(op) {
        return None;
    }
    let is_word = |inst: &Instruction, subtrahend: Option<&Operand>| match inst {
        Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ty,
            ..
        } => {
            *lhs == d1_operand
                && word_op(*ty, true) == Some(op)
                && subtrahend.is_none_or(|w| rhs == w)
        }
        _ => false,
    };
    // `d1 <u d2`: the bound is the word.
    if let Operand::Value(d2) = &bound
        && let Some(inst @ Instruction::BinOp { rhs, .. }) = facts.def(func, *d2)
        && is_word(inst, None)
    {
        let word_at = (*facts.defs.get(d2.0 as usize)?)?;
        return Some(Shape {
            word_at,
            word: *d2,
            op,
            x,
            y,
            w: rhs.clone(),
        });
    }
    // `d1 <u w`: the bound is the second subtrahend, and the word is the
    // subtraction of it from `d1`.
    let (word_at, word) = word_near(func, join_at, |inst| is_word(inst, Some(&bound)))?;
    Some(Shape {
        word_at,
        word,
        op,
        x,
        y,
        w: bound,
    })
}

/// The subtrahends added up first: `s = y + w`, `b1 = s <u y|w` (they
/// wrap), `b2 = x <u s`, and the word `x - s` ([`word_near`]). mini-gmp:
/// `b += cy; cy = b < cy; cy += a < b; r = a - b`.
///
/// With `y` or `w` a bit, `y + w` wraps only to 0, from which nothing
/// borrows: one of the two at most is set, and their join is the borrow of
/// `x - y - w`.
fn sub_of_sum_shape(
    func: &IrFunction,
    facts: &Facts,
    b1: Value,
    b2: Value,
    join_at: Position,
) -> Option<Shape> {
    let (s_operand, bound, ty1) = unsigned_less(func, facts, b1)?;
    let (s, y, w, add_ty) = binop(func, facts, &s_operand, IrBinOp::Add)?;
    if bound != y && bound != w {
        return None;
    }
    let s_operand = Operand::Value(s);
    let (x, subtrahend, ty2) = unsigned_less(func, facts, b2)?;
    if subtrahend != s_operand {
        return None;
    }
    let op = word_op(add_ty, true)?;
    if word_op(ty1, true) != Some(op) || word_op(ty2, true) != Some(op) {
        return None;
    }
    let (word_at, word) = word_near(func, join_at, |inst| {
        matches!(
            inst,
            Instruction::BinOp { op: IrBinOp::Sub, lhs, rhs, ty, .. }
                if *lhs == x && *rhs == s_operand && word_op(*ty, true) == Some(op)
        )
    })?;
    Some(Shape {
        word_at,
        word,
        op,
        x,
        y,
        w,
    })
}

/// What joins two partial carries of which one at most is set: their `|`,
/// and so their `+` and their `^`.
fn is_join(op: IrBinOp) -> bool {
    matches!(op, IrBinOp::Or | IrBinOp::Add | IrBinOp::Xor)
}

/// The join of two values that `inst` is: `(result, type, the two)`. A
/// `|`, `+` or `^`; or `c1 || c2` as the optimizer leaves it, the select
/// `c1 ? 1 : c2`.
fn join_of(inst: &Instruction) -> Option<(Value, IrType, Value, Value)> {
    match inst {
        Instruction::BinOp {
            dest,
            op,
            lhs: Operand::Value(first),
            rhs: Operand::Value(second),
            ty,
        } if is_join(*op) && is_integer(*ty) => Some((*dest, *ty, *first, *second)),
        Instruction::Select {
            dest,
            cond: Operand::Value(first),
            true_val,
            false_val: Operand::Value(second),
            ty,
        } if is_integer(*ty) && const_bit(true_val) == Some(true) => {
            Some((*dest, *ty, *first, *second))
        }
        _ => None,
    }
}

/// The two operations whose partial carries the instruction at `join_at`
/// joins, if it joins two.
fn shape_at(func: &IrFunction, facts: &Facts, join_at: Position) -> Option<Shape> {
    let (_, _, first, second) = join_of(&func.blocks[join_at.0].instructions[join_at.1])?;
    let first = partial_carry(func, facts, first)?;
    let second = partial_carry(func, facts, second)?;
    [(first, second), (second, first)]
        .into_iter()
        .find_map(|(c1, c2)| {
            add_shape(func, facts, c1, c2)
                .or_else(|| sub_shape(func, facts, c1, c2, join_at))
                .or_else(|| sub_of_sum_shape(func, facts, c1, c2, join_at))
        })
}

/// One full adder: where its word and its join are, and what they become.
struct Adder {
    /// `s2 = s1 + c` (or `d2 = d1 - c`, or `x - s`): the word.
    word_at: Position,
    word: Value,
    /// Where the halves go: in place of the word, which keeps its name; or,
    /// where the word stands behind the join in its block, in place of the
    /// join, the word under a new name (`behind`). Both results are then
    /// defined ahead of everything that read the old ones.
    at: Position,
    behind: bool,
    join_dest: Value,
    join_ty: IrType,
    op: CarryOp,
    args: [Operand; 3],
}

/// The adder whose carries the instruction at `join_at` joins, if it is
/// one: the shape, and an operand of it that is a bit.
fn adder_at(func: &IrFunction, facts: &mut Facts, join_at: Position) -> Option<Adder> {
    let shape = shape_at(func, facts, join_at)?;
    let args = shape.args(func, facts)?;
    let (join_dest, join_ty, ..) = join_of(&func.blocks[join_at.0].instructions[join_at.1])?;
    let behind = shape.word_at.0 == join_at.0 && shape.word_at.1 > join_at.1;
    Some(Adder {
        word_at: shape.word_at,
        word: shape.word,
        at: if behind { join_at } else { shape.word_at },
        behind,
        join_dest,
        join_ty,
        op: shape.op,
        args,
    })
}

/// A carry out read at `ty` is the carry out: 0 or 1 in a whole register,
/// as a comparison's result is, which is every integer of 32 bits or fewer.
fn narrow(ty: IrType) -> bool {
    is_integer(ty) && ty.size() <= 4
}

fn carry_halves(op: CarryOp, word: Value, out: Value, args: &[Operand; 3]) -> [Instruction; 2] {
    [
        Instruction::Intrinsic {
            dest: Some(word),
            op: IntrinsicOp::Carry(op),
            dest_ptr: None,
            args: args.to_vec(),
        },
        Instruction::Intrinsic {
            dest: Some(out),
            op: IntrinsicOp::Carry(op.partner()),
            dest_ptr: None,
            args: args.to_vec(),
        },
    ]
}

/// Put the first instruction of each group in place of the one at its
/// position and the others behind it, and read `renamed` values by their
/// new names everywhere.
fn install(
    func: &mut IrFunction,
    mut groups: Vec<(Position, Vec<Instruction>)>,
    renamed: &FxHashMap<u32, Value>,
) {
    // From the back of each block, so that an insertion moves nothing that
    // is still to be placed.
    groups.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for ((bi, ii), group) in groups {
        let block = &mut func.blocks[bi];
        let span = block.span_at(ii);
        for (offset, inst) in group.into_iter().enumerate() {
            if offset == 0 {
                block.instructions[ii] = inst;
            } else {
                block.insert_inst(ii + offset, inst, span);
            }
        }
    }
    if renamed.is_empty() {
        return;
    }
    let rename = |operand: &mut Operand| {
        if let Operand::Value(v) = operand
            && let Some(new) = renamed.get(&v.0)
        {
            *v = *new;
        }
    };
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(rename);
        }
        block.terminator.for_each_operand_mut(rename);
    }
}

/// Rewrite the full adders of `func`. Returns how many there were.
fn full_adders(func: &mut IrFunction) -> usize {
    // A function that compares no two unsigned words has no carry: nothing
    // is computed for it.
    let compares = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Cmp {
                    op: IrCmpOp::Ult | IrCmpOp::Ugt,
                    ty: IrType::U32 | IrType::U64,
                    ..
                }
            )
        })
    });
    if !compares {
        return 0;
    }
    let candidates: Vec<Position> = func
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(bi, block)| {
            block
                .instructions
                .iter()
                .enumerate()
                .filter(|(_, inst)| join_of(inst).is_some())
                .map(move |(ii, _)| (bi, ii))
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut facts = Facts::of(func);
    let mut adders: Vec<Adder> = Vec::new();
    let mut claimed: FxHashSet<Position> = FxHashSet::default();
    for join_at in candidates {
        if let Some(adder) = adder_at(func, &mut facts, join_at)
            // Two joins of the same adder's carries, spelled two ways: the
            // second one keeps its comparisons.
            && claimed.insert(adder.word_at)
        {
            adders.push(adder);
        }
    }
    if adders.is_empty() {
        return 0;
    }

    let mut next = func.sound_next_value_id();
    // The carry out that each join (and each conversion of one) is.
    let mut carry: FxHashMap<u32, Value> = FxHashMap::default();
    // What reads a join reads instead: the carry out at 32 bits or fewer,
    // and a conversion of it, made where the carry out is, above that.
    let mut renamed: FxHashMap<u32, Value> = FxHashMap::default();
    let outs: Vec<(Value, Value, Option<Instruction>)> = adders
        .iter()
        .map(|adder| {
            let mut fresh = || {
                next += 1;
                Value(next - 1)
            };
            // A word behind its join is computed where the join was, under
            // a new name; the old one is left for dead-code elimination.
            let word = if adder.behind {
                let word = fresh();
                renamed.insert(adder.word.0, word);
                word
            } else {
                adder.word
            };
            let out = fresh();
            carry.insert(adder.join_dest.0, out);
            if narrow(adder.join_ty) {
                renamed.insert(adder.join_dest.0, out);
                return (word, out, None);
            }
            let wide = fresh();
            renamed.insert(adder.join_dest.0, wide);
            let conversion = Instruction::Cast {
                dest: wide,
                src: Operand::Value(out),
                from_ty: IrType::U32,
                to_ty: adder.join_ty,
            };
            (word, out, Some(conversion))
        })
        .collect();
    func.next_value_id = next;

    // A conversion of a join to 32 bits or fewer is the carry out
    // (`unsigned char c = _addcarry_u64(...)`), and so is its `!= 0`
    // (`bool c = ...`); a wider conversion is one for the next link to
    // look through.
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                } if is_integer(*from_ty) && is_integer(*to_ty) => {
                    if let Some(&out) = carry.get(&src.0) {
                        carry.insert(dest.0, out);
                        if narrow(*to_ty) {
                            renamed.insert(dest.0, out);
                        }
                    }
                }
                Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(src),
                    rhs,
                    ..
                } if const_bit(rhs) == Some(false) => {
                    if let Some(&out) = carry.get(&src.0) {
                        carry.insert(dest.0, out);
                        renamed.insert(dest.0, out);
                    }
                }
                _ => {}
            }
        }
    }

    let count = adders.len();
    let groups = adders
        .into_iter()
        .zip(outs)
        .map(|(mut adder, (word, out, conversion))| {
            // The carry in of a link is the carry out of the one before.
            if let Operand::Value(c) = &adder.args[2]
                && let Some(&previous) = carry.get(&c.0)
            {
                adder.args[2] = Operand::Value(previous);
            }
            let mut group = carry_halves(adder.op, word, out, &adder.args).to_vec();
            group.extend(conversion);
            (adder.at, group)
        })
        .collect();
    install(func, groups, &renamed);
    count
}

/// The additions and subtractions a chain starts with: `s = x + y` whose
/// carry `s <u x` feeds nothing but the carry in of adders becomes the pair
/// with a constant 0 carry, and so does `d = x - y` standing ahead of its
/// borrow `x <u y` in the same block. Returns how many there were.
fn chain_heads(func: &mut IrFunction) -> usize {
    // How often each value is the carry in of an adder.
    let mut carried: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Intrinsic {
                op: IntrinsicOp::Carry(_),
                args,
                ..
            } = inst
                && let Some(Operand::Value(c)) = args.get(2)
            {
                *carried.entry(c.0).or_insert(0) += 1;
            }
        }
    }
    if carried.is_empty() {
        return 0;
    }
    let facts = Facts::of(func);
    let mut next = func.sound_next_value_id();
    let mut renamed: FxHashMap<u32, Value> = FxHashMap::default();
    let mut pairs: Vec<(Position, Vec<Instruction>)> = Vec::new();
    let mut claimed: FxHashSet<Position> = FxHashSet::default();
    // In the order of the function, so that the names given do not depend
    // on a hash.
    let mut carries: Vec<(Position, u32, u32)> = carried
        .iter()
        .filter_map(|(&c, &count)| Some(((*facts.defs.get(c as usize)?)?, c, count)))
        .collect();
    carries.sort_unstable();
    for ((bi, cmp_index), c, count) in carries {
        // Read by adders only: anything else would keep the comparison.
        if facts.uses.get(c as usize) != Some(&count) {
            continue;
        }
        let Some((a, b, ty)) = unsigned_less(func, &facts, Value(c)) else {
            continue;
        };
        // `s <u x` with `s = x + y`, or `x <u y` with `d = x - y` above it.
        let head = if let Some((s, x, y, add_ty)) = binop(func, &facts, &a, IrBinOp::Add)
            && (b == x || b == y)
            && let Some(op) = word_op(add_ty, false)
            && word_op(ty, false) == Some(op)
            && let Some(at) = facts.defs[s.0 as usize]
        {
            Some((at, s, op, [x, y, Operand::Const(IrConst::I8(0))]))
        } else if let Some(op) = word_op(ty, true) {
            func.blocks[bi].instructions[..cmp_index]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(ii, inst)| match inst {
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Sub,
                        lhs,
                        rhs,
                        ty: sub_ty,
                    } if *lhs == a && *rhs == b && word_op(*sub_ty, true) == Some(op) => Some((
                        (bi, ii),
                        *dest,
                        op,
                        [a.clone(), b.clone(), Operand::Const(IrConst::I8(0))],
                    )),
                    _ => None,
                })
        } else {
            None
        };
        let Some((at, word, op, args)) = head else {
            continue;
        };
        if !claimed.insert(at) {
            continue;
        }
        let out = Value(next);
        next += 1;
        renamed.insert(c, out);
        pairs.push((at, carry_halves(op, word, out, &args).to_vec()));
    }
    func.next_value_id = next;
    let count = pairs.len();
    if count != 0 {
        install(func, pairs, &renamed);
    }
    count
}

/// The additions and subtractions a chain ends with: `s = (x + y) + c`
/// whose own carry nobody reads (`r[3] = a[3] + b[3] + carry`) has no `|`
/// to be found by. Where `c` is the carry out of an adder and `x + y`
/// feeds nothing else, it is the word half alone: an `adc` that continues
/// the chain. Returns how many there were.
fn chain_tails(func: &mut IrFunction) -> usize {
    let mut facts = Facts::of(func);
    let carry_out = |func: &IrFunction, facts: &mut Facts, operand: &Operand| {
        let c = carry_of(func, facts, operand)?;
        let Operand::Value(v) = &c else {
            return None;
        };
        matches!(
            facts.def(func, *v),
            Some(Instruction::Intrinsic {
                op: IntrinsicOp::Carry(CarryOp { out: true, .. }),
                ..
            })
        )
        .then_some(c)
    };
    let mut tails: Vec<(Position, Instruction)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: op @ (IrBinOp::Add | IrBinOp::Sub),
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            let borrow = *op == IrBinOp::Sub;
            let Some(carry_op) = word_op(*ty, borrow) else {
                continue;
            };
            // `(x + y) + c` in either order; `(x - y) - c` in this one.
            let orders: &[(&Operand, &Operand)] = if borrow {
                &[(lhs, rhs)]
            } else {
                &[(lhs, rhs), (rhs, lhs)]
            };
            let found = orders.iter().find_map(|(first, carry_in)| {
                let (inner, x, y, inner_ty) = binop(func, &facts, first, *op)?;
                if inner_ty != *ty || !facts.read_once(inner) {
                    return None;
                }
                let c = carry_out(func, &mut facts, carry_in)?;
                Some([x, y, c])
            });
            if let Some(args) = found {
                tails.push((
                    (bi, ii),
                    Instruction::Intrinsic {
                        dest: Some(*dest),
                        op: IntrinsicOp::Carry(carry_op),
                        dest_ptr: None,
                        args: args.to_vec(),
                    },
                ));
            }
        }
    }
    let count = tails.len();
    for ((bi, ii), word) in tails {
        func.blocks[bi].instructions[ii] = word;
    }
    count
}

/// Rewrite the adders of `func`; `dce` says whether dead code may be
/// removed (the ends of a chain are found among what is left alive).
/// Returns the number of rewrites.
pub(crate) fn run(func: &mut IrFunction, dce: bool) -> usize {
    if func.is_declaration {
        return 0;
    }
    let full = full_adders(func);
    if full == 0 {
        return 0;
    }
    let sweep = |func: &mut IrFunction| {
        if dce {
            crate::passes::dce::eliminate_dead_code(func);
        }
    };
    sweep(func);
    let tails = chain_tails(func);
    if tails != 0 {
        sweep(func);
    }
    let heads = chain_heads(func);
    if heads != 0 {
        sweep(func);
    }
    full + tails + heads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    const U64: IrType = IrType::U64;

    fn param(dest: u32, idx: usize, ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: idx,
            ty,
        }
    }

    fn bin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty,
        }
    }

    fn cmp(dest: u32, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty,
        }
    }

    fn cast(dest: u32, src: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: v(src),
            from_ty,
            to_ty,
        }
    }

    fn v(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    fn zero() -> Operand {
        Operand::Const(IrConst::I8(0))
    }

    fn function(instructions: Vec<Instruction>, result: Operand) -> IrFunction {
        let mut func = IrFunction::new("adder".into(), IrType::U8, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(result)),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = func.sound_next_value_id();
        func
    }

    /// Appends one value that reads all of `values`, so that none of them
    /// is dead: the `^` of their conversions to 64 bits.
    fn reading(insts: &mut Vec<Instruction>, values: &[(u32, IrType)]) -> Operand {
        let mut all = None;
        for (i, &(value, ty)) in values.iter().enumerate() {
            let wide = 9000 + 2 * i as u32;
            insts.push(cast(wide, value, ty, U64));
            all = Some(match all {
                None => v(wide),
                Some(before) => {
                    insts.push(bin(wide + 1, IrBinOp::Xor, before, v(wide), U64));
                    v(wide + 1)
                }
            });
        }
        all.expect("something to read")
    }

    /// How many instructions of block 0 `matching` holds for.
    fn count(func: &IrFunction, matching: impl Fn(&Instruction) -> bool) -> usize {
        func.blocks[0]
            .instructions
            .iter()
            .filter(|inst| matching(inst))
            .count()
    }

    /// What an adder is spelled with and must be gone once it is one:
    /// comparisons of two values, `|`, `+` and `-`.
    fn spelling_left(func: &IrFunction) -> usize {
        count(func, |inst| match inst {
            Instruction::Cmp { op, .. } => *op != IrCmpOp::Ne,
            Instruction::BinOp { op, .. } => {
                matches!(op, IrBinOp::Or | IrBinOp::Add | IrBinOp::Sub)
            }
            _ => false,
        })
    }

    /// `_addcarry_u64(c, x, y, &r)` of the header as the optimizer leaves
    /// it, on values `base..`: x, y at `base`, `base + 1`; the carry in
    /// behind `!= 0` and a conversion; the result of the `|` at
    /// `base + 16`, its `unsigned char` at `base + 17`.
    fn header_adder(base: u32, carry_in: Operand, ty: IrType) -> Vec<Instruction> {
        vec![
            param(base, 0, ty),
            param(base + 1, 1, ty),
            bin(base + 10, IrBinOp::Add, v(base), v(base + 1), ty),
            cmp(base + 11, IrCmpOp::Ne, carry_in, zero(), IrType::U8),
            cast(base + 12, base + 11, IrType::I32, ty),
            bin(base + 13, IrBinOp::Add, v(base + 10), v(base + 12), ty),
            cmp(base + 14, IrCmpOp::Ult, v(base + 10), v(base), ty),
            cmp(base + 15, IrCmpOp::Ult, v(base + 13), v(base + 10), ty),
            bin(
                base + 16,
                IrBinOp::Or,
                v(base + 14),
                v(base + 15),
                IrType::I32,
            ),
            cast(base + 17, base + 16, IrType::I32, IrType::U8),
        ]
    }

    /// The carry halves of block 0: index, operation, destination, operands.
    fn halves(func: &IrFunction) -> Vec<(usize, CarryOp, Value, Vec<Operand>)> {
        func.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(i, inst)| match inst {
                Instruction::Intrinsic {
                    dest: Some(dest),
                    op: IntrinsicOp::Carry(op),
                    args,
                    ..
                } => Some((i, *op, *dest, args.clone())),
                _ => None,
            })
            .collect()
    }

    fn word_add(wide: bool) -> CarryOp {
        CarryOp {
            borrow: false,
            out: false,
            wide,
        }
    }

    #[test]
    fn the_header_adder_becomes_an_adjacent_pair() {
        for (ty, wide) in [(IrType::U64, true), (IrType::U32, false)] {
            let mut insts = vec![param(2, 2, IrType::U8)];
            insts.extend(header_adder(100, v(2), ty));
            let result = reading(&mut insts, &[(113, ty), (117, IrType::U8)]);
            let mut func = function(insts, result);
            assert_eq!(run(&mut func, true), 1, "{ty:?}");
            let found = halves(&func);
            assert_eq!(found.len(), 2);
            let (at, word, word_dest, args) = &found[0];
            let (out_at, out, out_dest, out_args) = &found[1];
            assert_eq!(*out_at, at + 1, "the carry is directly behind the word");
            assert_eq!(*word, word_add(wide));
            assert_eq!(*out, word.partner());
            // The word keeps its name.
            assert_eq!(*word_dest, Value(113));
            // The carry in is the comparison `c != 0`: `c` is a parameter,
            // and not known to be a bit.
            assert_eq!(*args, vec![v(100), v(101), v(111)]);
            assert_eq!(args, out_args);
            // The `unsigned char` of the `|` is the carry out itself, and
            // the comparisons, the `|` and the first addition are gone:
            // 3 parameters, `c != 0`, the pair, and the 3 that read it.
            assert_eq!(
                count(&func, |inst| matches!(
                    inst,
                    Instruction::Cast { src: Operand::Value(read), from_ty: IrType::U8, .. }
                        if read == out_dest
                )),
                1,
                "{func:#?}"
            );
            assert_eq!(spelling_left(&func), 0, "{func:#?}");
            assert_eq!(func.blocks[0].instructions.len(), 9, "{func:#?}");
        }
    }

    /// Either half alone: a carry nobody reads leaves the word, and a word
    /// nobody reads the carry.
    #[test]
    fn a_half_nobody_reads_is_gone() {
        for (kept, ty, out) in [(113, U64, false), (117, IrType::U8, true)] {
            let mut insts = vec![param(2, 2, IrType::U8)];
            insts.extend(header_adder(100, v(2), U64));
            let result = reading(&mut insts, &[(kept, ty)]);
            let mut func = function(insts, result);
            assert_eq!(run(&mut func, true), 1);
            let found = halves(&func);
            assert_eq!(found.len(), 1, "{func:#?}");
            assert_eq!(found[0].1.out, out);
            assert_eq!(found[0].3, vec![v(100), v(101), v(111)]);
            assert_eq!(spelling_left(&func), 0, "{func:#?}");
        }
    }

    /// The carry of one link is the carry operand of the next, behind the
    /// conversions between them; a chain of three is three pairs and
    /// nothing else.
    #[test]
    fn a_chain_passes_the_carry_itself() {
        let mut insts = vec![param(2, 2, IrType::U8)];
        insts.extend(header_adder(100, v(2), U64));
        insts.extend(header_adder(200, v(117), U64));
        insts.extend(header_adder(300, v(217), U64));
        let result = reading(
            &mut insts,
            &[(113, U64), (213, U64), (313, U64), (317, IrType::U8)],
        );
        let mut func = function(insts, result);
        assert_eq!(run(&mut func, true), 3);
        let found = halves(&func);
        assert_eq!(found.len(), 6);
        for link in 1..3 {
            let previous_out = found[2 * link - 1].2;
            let base = 100 * (link as u32 + 1);
            assert_eq!(
                found[2 * link].3,
                vec![v(base), v(base + 1), Operand::Value(previous_out)],
                "link {link} reads the carry out before it"
            );
        }
        // 7 parameters, the `c != 0` of the first link, 6 halves, and the
        // 7 that read them.
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
        assert_eq!(func.blocks[0].instructions.len(), 21, "{func:#?}");
    }

    /// A chain that starts with a constant 0 carry starts with a plain
    /// addition; that addition becomes the pair with a constant carry.
    #[test]
    fn the_head_of_a_chain_is_an_add_and_its_carry() {
        let mut insts = vec![
            param(0, 0, U64),
            param(1, 1, U64),
            bin(10, IrBinOp::Add, v(0), v(1), U64),
            cmp(11, IrCmpOp::Ult, v(10), v(1), U64),
            cast(17, 11, IrType::I32, IrType::U8),
        ];
        insts.extend(header_adder(200, v(17), U64));
        let result = reading(&mut insts, &[(10, U64), (213, U64), (217, IrType::U8)]);
        let mut func = function(insts, result);
        assert_eq!(run(&mut func, true), 2);
        let found = halves(&func);
        assert_eq!(found.len(), 4, "{func:#?}");
        assert_eq!(found[0].1, word_add(true));
        assert_eq!(found[0].2, Value(10), "the addition keeps its name");
        assert_eq!(found[0].3, vec![v(0), v(1), zero()]);
        assert_eq!(found[1].0, found[0].0 + 1);
        assert_eq!(found[2].3[2], Operand::Value(found[1].2));
        assert_eq!(
            count(&func, |inst| matches!(inst, Instruction::Cmp { .. })),
            0,
            "no comparison is left: {func:#?}"
        );
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
    }

    /// `r[1] = a[1] + b[1] + carry` with the carry of a full adder and no
    /// carry out of its own is the word half alone.
    #[test]
    fn the_tail_of_a_chain_is_the_word_alone() {
        for (op, borrow) in [(IrBinOp::Add, false), (IrBinOp::Sub, true)] {
            let mut insts = if borrow {
                header_subtractor(cmp(15, IrCmpOp::Ult, v(10), v(12), U64))
            } else {
                // On the values the subtractor uses: x, y at 0, 1, the
                // carry parameter at 2, the word at 13 and the `|` at 16.
                let mut insts = vec![param(2, 2, IrType::U8)];
                insts.extend(header_adder(0, v(2), U64));
                insts
            };
            insts.extend([
                param(40, 3, U64),
                param(41, 4, U64),
                bin(42, op, v(40), v(41), U64),
                cast(43, 16, IrType::I32, U64),
                bin(44, op, v(42), v(43), U64),
                bin(45, IrBinOp::Xor, v(44), v(13), U64),
            ]);
            let mut func = function(insts, v(45));
            assert_eq!(run(&mut func, true), 2, "{op:?}: {func:#?}");
            let found = halves(&func);
            assert_eq!(found.len(), 3, "{op:?}: {func:#?}");
            let (_, tail, tail_dest, tail_args) = &found[2];
            assert_eq!(
                *tail,
                CarryOp {
                    borrow,
                    out: false,
                    wide: true
                }
            );
            assert_eq!(*tail_dest, Value(44));
            assert_eq!(
                *tail_args,
                vec![v(40), v(41), Operand::Value(found[1].2)],
                "the tail reads the carry out of the adder before it"
            );
            assert!(
                !func.blocks[0]
                    .instructions
                    .iter()
                    .any(|inst| matches!(inst, Instruction::BinOp { op: o, .. } if *o == op)),
                "no plain operation is left: {func:#?}"
            );
        }
    }

    /// A sum with a carry in that is no adder's carry out stays two
    /// additions: the peephole has the shorter form for a comparison.
    #[test]
    fn a_sum_with_a_comparison_added_is_no_tail() {
        let mut insts = vec![param(2, 2, IrType::U8)];
        insts.extend(header_adder(100, v(2), U64));
        insts.extend([
            param(40, 3, U64),
            param(41, 4, U64),
            bin(42, IrBinOp::Add, v(40), v(41), U64),
            cmp(46, IrCmpOp::Ult, v(40), v(41), U64),
            cast(43, 46, IrType::I32, U64),
            bin(44, IrBinOp::Add, v(42), v(43), U64),
            bin(45, IrBinOp::Xor, v(44), v(113), U64),
            cast(47, 117, IrType::U8, U64),
            bin(48, IrBinOp::Xor, v(45), v(47), U64),
        ]);
        let mut func = function(insts, v(48));
        assert_eq!(run(&mut func, true), 1);
        assert_eq!(halves(&func).len(), 2);
    }

    /// The same head with its carry read by something else keeps the
    /// comparison: the pair would add a `setc` to it.
    #[test]
    fn a_head_whose_carry_is_read_elsewhere_stays() {
        let mut insts = vec![
            param(0, 0, U64),
            param(1, 1, U64),
            bin(10, IrBinOp::Add, v(0), v(1), U64),
            cmp(11, IrCmpOp::Ult, v(10), v(1), U64),
            cast(17, 11, IrType::I32, IrType::U8),
        ];
        insts.extend(header_adder(200, v(17), U64));
        let result = reading(
            &mut insts,
            &[(11, IrType::I32), (213, U64), (217, IrType::U8)],
        );
        let mut func = function(insts, result);
        assert_eq!(run(&mut func, true), 1);
        assert_eq!(halves(&func).len(), 2);
        assert_eq!(
            count(&func, |inst| matches!(
                inst,
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    ..
                }
            )),
            1,
            "the first addition is an addition still: {func:#?}"
        );
    }

    fn header_subtractor(second: Instruction) -> Vec<Instruction> {
        vec![
            param(0, 0, U64),
            param(1, 1, U64),
            param(2, 2, IrType::U8),
            bin(10, IrBinOp::Sub, v(0), v(1), U64),
            cmp(11, IrCmpOp::Ne, v(2), zero(), IrType::U8),
            cast(12, 11, IrType::I32, U64),
            bin(13, IrBinOp::Sub, v(10), v(12), U64),
            cmp(14, IrCmpOp::Ult, v(0), v(1), U64),
            second,
            bin(16, IrBinOp::Or, v(14), v(15), IrType::I32),
        ]
    }

    #[test]
    fn both_spellings_of_the_borrow_are_a_borrow_pair() {
        for second in [
            // d1 <u c
            cmp(15, IrCmpOp::Ult, v(10), v(12), U64),
            // d2 >u d1
            cmp(15, IrCmpOp::Ugt, v(13), v(10), U64),
        ] {
            let mut insts = header_subtractor(second);
            let result = reading(&mut insts, &[(13, U64), (16, IrType::I32)]);
            let mut func = function(insts, result);
            assert_eq!(run(&mut func, true), 1, "{func:#?}");
            let found = halves(&func);
            assert_eq!(found.len(), 2);
            assert!(found[0].1.borrow && !found[0].1.out && found[1].1.out);
            assert_eq!(found[0].2, Value(13));
            assert_eq!(found[0].3, vec![v(0), v(1), v(11)]);
            assert_eq!(found[1].0, found[0].0 + 1);
            assert_eq!(spelling_left(&func), 0, "{func:#?}");
        }
    }

    /// `d1 <u c` does not read the word. One that stands behind the `|`
    /// is computed where the `|` was, under a new name, so that the carry
    /// out is there for what read the `|` ahead of the word.
    #[test]
    fn a_word_behind_the_join_moves_up_to_it() {
        let mut insts = header_subtractor(cmp(15, IrCmpOp::Ult, v(10), v(12), U64));
        let word = insts.remove(6);
        insts.push(cast(19, 16, IrType::I32, U64));
        insts.push(word);
        insts.push(bin(20, IrBinOp::Xor, v(13), v(19), U64));
        let mut func = function(insts, v(20));
        assert_eq!(run(&mut func, true), 1);
        let found = halves(&func);
        assert_eq!(found.len(), 2, "{func:#?}");
        let (at, op, word, args) = &found[0];
        let (out_at, _, out, _) = &found[1];
        assert!(op.borrow && !op.out);
        assert_ne!(*word, Value(13), "the word has a new name");
        assert_eq!(*args, vec![v(0), v(1), v(11)]);
        assert_eq!(*out_at, at + 1);
        let insts = &func.blocks[0].instructions;
        // 3 parameters, `c != 0`, the pair, and behind it the two readers.
        assert_eq!(insts.len(), 8, "{func:#?}");
        assert!(
            matches!(&insts[6], Instruction::Cast { src, .. } if *src == Operand::Value(*out)),
            "{func:#?}"
        );
        assert!(
            matches!(&insts[7], Instruction::BinOp { lhs, .. } if *lhs == Operand::Value(*word)),
            "{func:#?}"
        );
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
    }

    /// The subtrahends added up first (mini-gmp's `mpn_sub_n`): `s = y + c`,
    /// `s <u c`, `x <u s`, and the word `x - s` behind their sum.
    #[test]
    fn a_subtraction_of_the_summed_subtrahends_is_a_borrow_pair() {
        let subtractor = || {
            vec![
                param(0, 0, U64),
                param(1, 1, U64),
                param(2, 2, U64),
                bin(3, IrBinOp::And, v(2), Operand::Const(IrConst::I64(1)), U64),
                bin(10, IrBinOp::Add, v(1), v(3), U64),
                cmp(11, IrCmpOp::Ult, v(10), v(3), U64),
                cast(12, 11, IrType::I32, U64),
                cmp(14, IrCmpOp::Ult, v(0), v(10), U64),
                cast(15, 14, IrType::I32, U64),
                bin(16, IrBinOp::Add, v(12), v(15), U64),
                bin(13, IrBinOp::Sub, v(0), v(10), U64),
                bin(17, IrBinOp::Xor, v(16), v(13), U64),
            ]
        };
        let mut func = function(subtractor(), v(17));
        assert_eq!(run(&mut func, true), 1, "{func:#?}");
        let found = halves(&func);
        assert!(found[0].1.borrow);
        assert_eq!(found[0].3, vec![v(0), v(1), v(3)]);
        wide_carry(&func, 17);
        // 3 parameters, the bit, the pair, its conversion and the `^`.
        assert_eq!(func.blocks[0].instructions.len(), 8, "{func:#?}");

        // The word ahead of the join keeps its place and its name.
        let mut insts = subtractor();
        let word = insts.remove(10);
        insts.insert(5, word);
        let mut func = function(insts, v(17));
        assert_eq!(run(&mut func, true), 1, "{func:#?}");
        assert_eq!(halves(&func)[0].2, Value(13));
        assert_eq!(func.blocks[0].instructions.len(), 8, "{func:#?}");

        let miss = |what: &str, edit: &dyn Fn(&mut Vec<Instruction>)| {
            let mut insts = subtractor();
            edit(&mut insts);
            assert_eq!(run(&mut function(insts, v(17)), true), 0, "{what}");
        };
        miss("the difference the other way round", &|insts| {
            insts[10] = bin(13, IrBinOp::Sub, v(10), v(0), U64);
        });
        miss("a borrow against one subtrahend", &|insts| {
            insts[7] = cmp(14, IrCmpOp::Ult, v(0), v(1), U64);
        });
        miss("a wrap tested against the minuend", &|insts| {
            insts[5] = cmp(11, IrCmpOp::Ult, v(10), v(0), U64);
        });
        miss("two words subtracted", &|insts| {
            insts[3] = bin(3, IrBinOp::And, v(2), Operand::Const(IrConst::I64(3)), U64);
        });
    }

    /// `x + y + (w & 1)` with its two partial carries converted to 64 bits
    /// and joined by `join` at 64 bits; the join is `%16`, and `%17` reads
    /// it and the word.
    fn wide_adder(join: IrBinOp) -> Vec<Instruction> {
        vec![
            param(0, 0, U64),
            param(1, 1, U64),
            param(2, 2, U64),
            bin(3, IrBinOp::And, v(2), Operand::Const(IrConst::I64(1)), U64),
            bin(10, IrBinOp::Add, v(0), v(1), U64),
            cmp(11, IrCmpOp::Ult, v(10), v(0), U64),
            cast(12, 11, IrType::I32, U64),
            bin(13, IrBinOp::Add, v(10), v(3), U64),
            cmp(14, IrCmpOp::Ult, v(13), v(10), U64),
            cast(15, 14, IrType::I32, U64),
            bin(16, join, v(12), v(15), U64),
            bin(17, IrBinOp::Xor, v(16), v(13), U64),
        ]
    }

    /// The conversion of the carry out that stands directly behind the
    /// pair, if `reader` reads it: `(the carry out, the conversion)`.
    fn wide_carry(func: &IrFunction, reader: u32) -> (Value, Value) {
        let found = halves(func);
        assert_eq!(found.len(), 2, "{func:#?}");
        let (out_at, _, out, _) = &found[1];
        let Instruction::Cast {
            dest,
            src,
            from_ty: IrType::U32,
            to_ty: IrType::U64,
        } = &func.blocks[0].instructions[out_at + 1]
        else {
            panic!("no conversion behind the carry out: {func:#?}");
        };
        assert_eq!(*src, Operand::Value(*out));
        let mut read = false;
        for inst in &func.blocks[0].instructions {
            if inst.dest() == Some(Value(reader)) {
                inst.for_each_used_value(|used| read |= used == dest.0);
            }
        }
        assert!(read, "%{reader} reads the conversion: {func:#?}");
        (*out, *dest)
    }

    /// A 64-bit `|` of converted comparisons (`carry = (s < a) | (t < s)`
    /// in `unsigned long long`) is a conversion of the carry out, made
    /// where the carry out is: a zero extension, and so a 32-bit move
    /// where the old one, from a comparison's `int`, was a sign extension.
    #[test]
    fn a_wide_or_becomes_a_conversion() {
        let mut func = function(wide_adder(IrBinOp::Or), v(17));
        assert_eq!(run(&mut func, true), 1);
        let found = halves(&func);
        assert_eq!(found[0].3, vec![v(0), v(1), v(3)]);
        wide_carry(&func, 17);
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
    }

    /// Two carries of which one at most is set add up to their `|`, and
    /// their `^` is it too (`cy = s < a; ...; cy += t < s`).
    #[test]
    fn a_sum_and_an_exclusive_or_of_the_carries_are_the_carry() {
        for join in [IrBinOp::Add, IrBinOp::Xor] {
            let mut func = function(wide_adder(join), v(17));
            assert_eq!(run(&mut func, true), 1, "{join:?}");
            assert_eq!(halves(&func)[0].3, vec![v(0), v(1), v(3)]);
            wide_carry(&func, 17);
            // Only the `^` that reads the results is left.
            assert_eq!(
                count(&func, |inst| matches!(
                    inst,
                    Instruction::BinOp { .. } | Instruction::Cmp { .. }
                )),
                2,
                "{join:?}: {func:#?}"
            );
        }
    }

    /// ... and of two that may both be set (a sum of three words carries
    /// twice) neither is: `+` counts to two, `^` says no carry.
    #[test]
    fn carries_of_three_words_are_not_joined() {
        for join in [IrBinOp::Or, IrBinOp::Add, IrBinOp::Xor] {
            let mut insts = wide_adder(join);
            // The third operand is the word itself, not its lowest bit.
            insts[7] = bin(13, IrBinOp::Add, v(10), v(2), U64);
            let mut func = function(insts, v(17));
            assert_eq!(run(&mut func, true), 0, "{join:?}");
        }
    }

    /// Whichever of the three operands is the bit, it is the carry in:
    /// `(x + c) + y` (mini-gmp adds the carry first) and `(c + y) + w`.
    #[test]
    fn the_bit_may_be_any_operand_of_an_addition() {
        // %3 is the bit; the first addition reads `first`, the second adds
        // `third`, and the comparisons are against the operand named.
        for (first, third, want) in [
            // (x + c) + y
            ((v(0), v(3)), v(1), vec![v(0), v(1), v(3)]),
            // (c + x) + y
            ((v(3), v(0)), v(1), vec![v(0), v(1), v(3)]),
            // (x + y) + c: the control.
            ((v(0), v(1)), v(3), vec![v(0), v(1), v(3)]),
        ] {
            let mut insts = wide_adder(IrBinOp::Add);
            insts[4] = bin(10, IrBinOp::Add, first.0.clone(), first.1.clone(), U64);
            insts[5] = cmp(11, IrCmpOp::Ult, v(10), first.1.clone(), U64);
            insts[7] = bin(13, IrBinOp::Add, v(10), third.clone(), U64);
            insts[8] = cmp(14, IrCmpOp::Ult, v(13), third.clone(), U64);
            let mut func = function(insts, v(17));
            assert_eq!(run(&mut func, true), 1, "{first:?} + {third:?}");
            assert_eq!(halves(&func)[0].3, want, "{first:?} + {third:?}");
        }
    }

    /// Of a subtraction either subtrahend may be the bit, and the minuend
    /// may not: `1 - y - w` borrows twice.
    #[test]
    fn the_bit_may_be_either_subtrahend_and_not_the_minuend() {
        let subtractor = |minuend: u32, first: u32, second: u32| {
            vec![
                param(0, 0, U64),
                param(1, 1, U64),
                param(2, 2, U64),
                bin(3, IrBinOp::And, v(2), Operand::Const(IrConst::I64(1)), U64),
                bin(10, IrBinOp::Sub, v(minuend), v(first), U64),
                cmp(11, IrCmpOp::Ult, v(minuend), v(first), U64),
                cast(12, 11, IrType::I32, U64),
                bin(13, IrBinOp::Sub, v(10), v(second), U64),
                cmp(14, IrCmpOp::Ult, v(10), v(second), U64),
                cast(15, 14, IrType::I32, U64),
                bin(16, IrBinOp::Add, v(12), v(15), U64),
                bin(17, IrBinOp::Xor, v(16), v(13), U64),
            ]
        };
        // (x - y) - c, and (x - c) - y: both are x - y - c.
        for (first, second) in [(1, 3), (3, 1)] {
            let mut func = function(subtractor(0, first, second), v(17));
            assert_eq!(run(&mut func, true), 1, "(x - %{first}) - %{second}");
            let found = halves(&func);
            assert!(found[0].1.borrow);
            assert_eq!(found[0].3, vec![v(0), v(1), v(3)]);
            wide_carry(&func, 17);
        }
        // (c - x) - y.
        let mut func = function(subtractor(3, 0, 1), v(17));
        assert_eq!(run(&mut func, true), 0, "{func:#?}");
    }

    /// The first borrow spelled as the second one may be: `d1 >u x`, the
    /// difference is the greater (GMP's `sub_n`).
    #[test]
    fn a_first_borrow_may_say_the_difference_grew() {
        let mut insts = header_subtractor(cmp(15, IrCmpOp::Ugt, v(13), v(10), U64));
        insts[7] = cmp(14, IrCmpOp::Ugt, v(10), v(0), U64);
        let result = reading(&mut insts, &[(13, U64), (16, IrType::I32)]);
        let mut func = function(insts, result);
        assert_eq!(run(&mut func, true), 1, "{func:#?}");
        let found = halves(&func);
        assert!(found[0].1.borrow);
        assert_eq!(found[0].3, vec![v(0), v(1), v(11)]);
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
        // Against the subtrahend it says nothing of the kind.
        let mut insts = header_subtractor(cmp(15, IrCmpOp::Ugt, v(13), v(10), U64));
        insts[7] = cmp(14, IrCmpOp::Ugt, v(10), v(1), U64);
        let result = reading(&mut insts, &[(13, U64), (16, IrType::I32)]);
        assert_eq!(run(&mut function(insts, result), true), 0);
    }

    /// `c1 || c2` as the optimizer leaves it, `c1 ? 1 : c2`, and the
    /// `!= 0` a `bool` makes of it.
    #[test]
    fn a_logical_or_of_the_carries_is_the_carry() {
        let mut insts = vec![param(2, 2, IrType::U8)];
        insts.extend(header_adder(100, v(2), U64));
        // In place of the `|` and its conversion.
        insts.truncate(9);
        insts.push(cast(120, 115, IrType::U8, IrType::I64));
        insts.push(Instruction::Select {
            dest: Value(116),
            cond: v(114),
            true_val: Operand::Const(IrConst::I64(1)),
            false_val: v(120),
            ty: IrType::I64,
        });
        insts.push(cmp(
            117,
            IrCmpOp::Ne,
            v(116),
            Operand::Const(IrConst::I64(0)),
            IrType::I64,
        ));
        let result = reading(&mut insts, &[(113, U64), (117, IrType::U8)]);
        let mut func = function(insts, result);
        assert_eq!(run(&mut func, true), 1, "{func:#?}");
        let found = halves(&func);
        assert_eq!(found.len(), 2, "{func:#?}");
        assert_eq!(found[0].3, vec![v(100), v(101), v(111)]);
        // The `bool` is the carry out: nothing converts it.
        let out = found[1].2;
        assert_eq!(
            count(&func, |inst| matches!(
                inst,
                Instruction::Cast { src: Operand::Value(read), from_ty: IrType::U8, .. }
                    if *read == out
            )),
            1,
            "{func:#?}"
        );
        assert_eq!(
            count(&func, |inst| matches!(inst, Instruction::Select { .. })),
            0
        );
        assert_eq!(spelling_left(&func), 0, "{func:#?}");
        // A select of anything else is no join.
        let mut insts = vec![param(2, 2, IrType::U8)];
        insts.extend(header_adder(100, v(2), U64));
        insts.truncate(9);
        insts.push(cast(120, 115, IrType::U8, IrType::I64));
        insts.push(Instruction::Select {
            dest: Value(116),
            cond: v(114),
            true_val: Operand::Const(IrConst::I64(2)),
            false_val: v(120),
            ty: IrType::I64,
        });
        let result = reading(&mut insts, &[(113, U64), (116, IrType::I64)]);
        assert_eq!(run(&mut function(insts, result), true), 0);
    }

    /// What must not match.
    #[test]
    fn near_misses_are_left_alone() {
        let adder = |edit: &dyn Fn(&mut Vec<Instruction>)| {
            let mut insts = vec![param(2, 2, IrType::U8)];
            insts.extend(header_adder(100, v(2), U64));
            edit(&mut insts);
            run(&mut function(insts, v(117)), true)
        };
        assert_eq!(adder(&|_| {}), 1, "the control");
        // A carry in that is a whole word: `__builtin_addcll(a, b, n, &o)`.
        assert_eq!(
            adder(&|insts| insts[5] = param(112, 3, U64)),
            0,
            "word-sized carry in"
        );
        assert_eq!(
            adder(&|insts| insts[7] = cmp(114, IrCmpOp::Ult, v(110), v(112), U64)),
            0,
            "a first comparison against neither addend"
        );
        assert_eq!(
            adder(&|insts| insts[8] = cmp(115, IrCmpOp::Slt, v(113), v(110), U64)),
            0,
            "a signed comparison"
        );
        assert_eq!(
            adder(&|insts| insts[8] = cmp(115, IrCmpOp::Ult, v(113), v(110), IrType::U32)),
            0,
            "a comparison at another width"
        );
        assert_eq!(
            adder(&|insts| insts.push(Instruction::Copy {
                dest: Value(118),
                src: v(114),
            })),
            0,
            "a partial carry with a second reader"
        );
        assert_eq!(
            adder(&|insts| insts[9] = bin(116, IrBinOp::And, v(114), v(115), IrType::I32)),
            0,
            "an `&` of the partial carries"
        );
        // 16-bit and signed words have no `adc` here.
        for ty in [IrType::U16, IrType::I32, IrType::I64] {
            let mut insts = vec![param(2, 2, IrType::U8)];
            insts.extend(header_adder(100, v(2), ty));
            assert_eq!(run(&mut function(insts, v(117)), true), 0, "{ty:?}");
        }
    }

    /// The bits: comparisons, and what is made of bits only.
    #[test]
    fn booleans_are_bits_and_what_is_made_of_bits() {
        let insts = vec![
            param(0, 0, U64),
            param(1, 1, U64),
            cmp(10, IrCmpOp::Ult, v(0), v(1), U64),
            cast(11, 10, IrType::I32, U64),
            bin(12, IrBinOp::And, v(0), Operand::Const(IrConst::I64(1)), U64),
            bin(13, IrBinOp::Or, v(11), v(12), U64),
            bin(14, IrBinOp::Or, v(11), v(0), U64),
            bin(15, IrBinOp::Add, v(11), v(12), U64),
            bin(16, IrBinOp::And, v(0), Operand::Const(IrConst::I64(3)), U64),
            Instruction::Copy {
                dest: Value(17),
                src: v(13),
            },
            bin(
                18,
                IrBinOp::Xor,
                v(17),
                Operand::Const(IrConst::I64(1)),
                U64,
            ),
            bin(
                19,
                IrBinOp::Xor,
                v(17),
                Operand::Const(IrConst::I64(2)),
                U64,
            ),
            bin(20, IrBinOp::Or, v(19), v(10), U64),
        ];
        let func = function(insts, v(17));
        let set = booleans(&func, &Facts::of(&func));
        for bit in [10, 11, 12, 13, 17, 18] {
            assert!(set.contains(&bit), "%{bit} is a bit");
        }
        for word in [0, 1, 14, 15, 16, 19, 20] {
            assert!(!set.contains(&word), "%{word} is not");
        }
    }

    /// A carry that goes round a loop: `c = phi(0, out)` is a bit because
    /// everything it can be is one, and it is the carry in.
    #[test]
    fn a_carry_round_a_loop_is_a_bit() {
        let mut func = IrFunction::new("limbs".into(), IrType::U8, vec![], false);
        let block = |label: u32, instructions, terminator| BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        };
        func.blocks.push(block(
            0,
            vec![param(0, 0, U64), param(1, 1, U64)],
            Terminator::Branch(BlockId(1)),
        ));
        let mut body = vec![Instruction::Phi {
            dest: Value(50),
            ty: IrType::U8,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), BlockId(0)),
                (v(117), BlockId(1)),
            ],
        }];
        body.extend(
            header_adder(100, v(50), U64)
                .into_iter()
                .skip(2)
                .map(|mut inst| {
                    inst.for_each_operand_mut(|operand| {
                        if *operand == v(100) {
                            *operand = v(0);
                        } else if *operand == v(101) {
                            *operand = v(1);
                        }
                    });
                    inst
                }),
        );
        func.blocks.push(block(
            1,
            body,
            Terminator::CondBranch {
                cond: v(113),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        func.blocks
            .push(block(2, vec![], Terminator::Return(Some(v(50)))));
        func.next_value_id = func.sound_next_value_id();
        assert_eq!(run(&mut func, true), 1);
        let body = &func.blocks[1].instructions;
        let (word, out) = (&body[1], &body[2]);
        let Instruction::Intrinsic {
            op: IntrinsicOp::Carry(op),
            args,
            ..
        } = word
        else {
            panic!("{body:#?}");
        };
        assert_eq!(*op, word_add(true));
        assert_eq!(*args, vec![v(0), v(1), v(50)], "the phi is the carry in");
        let Instruction::Intrinsic {
            dest: Some(carry), ..
        } = out
        else {
            panic!("{body:#?}");
        };
        assert!(
            matches!(&body[0], Instruction::Phi { incoming, .. }
                if incoming[1].0 == Operand::Value(*carry)),
            "and the carry out is what comes round: {body:#?}"
        );
        assert_eq!(body.len(), 3);
    }

    /// One trip of a limb loop over `x = %0`, `y = %1`: `%50` goes round,
    /// starting at `start` and continuing as the `join` of the two partial
    /// carries of `(x + y) + %50`. Returns the function and the body.
    fn counted_loop(start: Operand, join: IrBinOp) -> IrFunction {
        let mut func = IrFunction::new("limbs".into(), U64, vec![], false);
        let block = |label: u32, instructions, terminator| BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        };
        func.blocks.push(block(
            0,
            vec![param(0, 0, U64), param(1, 1, U64), param(2, 2, U64)],
            Terminator::Branch(BlockId(1)),
        ));
        func.blocks.push(block(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(50),
                    ty: U64,
                    incoming: vec![(start, BlockId(0)), (v(16), BlockId(1))],
                },
                bin(10, IrBinOp::Add, v(0), v(1), U64),
                cmp(11, IrCmpOp::Ult, v(10), v(0), U64),
                cast(12, 11, IrType::I32, U64),
                bin(13, IrBinOp::Add, v(10), v(50), U64),
                cmp(14, IrCmpOp::Ult, v(13), v(10), U64),
                cast(15, 14, IrType::I32, U64),
                bin(16, join, v(12), v(15), U64),
            ],
            Terminator::CondBranch {
                cond: v(13),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        func.blocks
            .push(block(2, vec![], Terminator::Return(Some(v(50)))));
        func.next_value_id = func.sound_next_value_id();
        func
    }

    /// The sum of the two carries goes round the loop and is the third
    /// operand of the next trip: it is a bit because the third operand is
    /// one, which is the sum itself. Started at 0 or 1 it stands; started
    /// at a word it falls, and the loop counts carries (0, 1 or 2 a trip).
    #[test]
    fn a_sum_of_carries_round_a_loop_is_a_bit_if_it_starts_as_one() {
        for start in [0, 1] {
            let mut func = counted_loop(Operand::Const(IrConst::I64(start)), IrBinOp::Add);
            assert!(booleans(&func, &Facts::of(&func)).contains(&16));
            assert_eq!(run(&mut func, true), 1, "started at {start}");
            let body = &func.blocks[1].instructions;
            let Instruction::Intrinsic {
                op: IntrinsicOp::Carry(op),
                args,
                ..
            } = &body[1]
            else {
                panic!("{body:#?}");
            };
            assert_eq!(*op, word_add(true));
            assert_eq!(*args, vec![v(0), v(1), v(50)]);
            // The phi, the pair and the conversion that comes round.
            assert_eq!(body.len(), 4, "{body:#?}");
            let Instruction::Cast { dest: wide, .. } = &body[3] else {
                panic!("{body:#?}");
            };
            assert!(
                matches!(&body[0], Instruction::Phi { incoming, .. }
                    if incoming[1].0 == Operand::Value(*wide)),
                "{body:#?}"
            );
        }
        for start in [Operand::Const(IrConst::I64(2)), v(2)] {
            let mut func = counted_loop(start.clone(), IrBinOp::Add);
            let bits = booleans(&func, &Facts::of(&func));
            assert!(!bits.contains(&16) && !bits.contains(&50), "{start:?}");
            assert_eq!(run(&mut func, true), 0, "started at {start:?}");
        }
        // With `|` the join is a bit whatever it joins, and the carry in is
        // one from the second trip on; the first trip adds the word.
        let mut func = counted_loop(v(2), IrBinOp::Or);
        assert_eq!(run(&mut func, true), 0);
    }

    /// The table of [`CarryOp::evaluate`] against 128-bit arithmetic.
    #[test]
    fn evaluate_is_the_three_term_sum() {
        let samples = [
            0u64,
            1,
            2,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_ffff,
            0x1_0000_0000,
            u64::MAX - 1,
            u64::MAX,
        ];
        for wide in [false, true] {
            let bits = if wide { 64 } else { 32 };
            let mask = if wide {
                u128::from(u64::MAX)
            } else {
                0xffff_ffff
            };
            for &x in &samples {
                for &y in &samples {
                    for c in [false, true] {
                        let (xm, ym) = (u128::from(x) & mask, u128::from(y) & mask);
                        let add = CarryOp {
                            borrow: false,
                            out: false,
                            wide,
                        };
                        let sum = xm + ym + u128::from(c);
                        assert_eq!(
                            add.evaluate(x, y, c),
                            ((sum & mask) as u64, sum >> bits != 0)
                        );
                        let sub = CarryOp {
                            borrow: true,
                            out: false,
                            wide,
                        };
                        let diff = xm.wrapping_sub(ym).wrapping_sub(u128::from(c));
                        assert_eq!(
                            sub.evaluate(x, y, c),
                            ((diff & mask) as u64, xm < ym + u128::from(c))
                        );
                    }
                }
            }
        }
    }
}
