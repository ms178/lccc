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
//!   carries is set, and their `|` is their sum. (A carry in that is not
//!   proven a bit is left alone: `__builtin_addcll(a, b, n, &o)` adds any
//!   `n`, and `adc` adds one bit.) `c` is the bit itself, found behind the
//!   conversions and the `!= 0` the sources write.
//! * The new instructions read `x`, `y` and `c`, which the old ones read
//!   (transitively) at the same place or earlier, so every definition still
//!   dominates its uses. `out` moves up to behind the word, which dominated
//!   the `|` -- by construction where the `|` reads a comparison of the word
//!   (every addition, and the subtraction spelled `d2 >u d1`), and checked
//!   where it does not (the subtraction spelled `d1 <u c`: the word must sit
//!   earlier in the block of the `|`).
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
        self.bools
            .get_or_insert_with(|| booleans(func))
            .contains(&v.0)
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
fn booleans(func: &IrFunction) -> FxHashSet<u32> {
    // What each candidate reads, and whether it reads a constant other
    // than 0 or 1.
    let mut reads: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut struck: Vec<u32> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
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
    let mut set: FxHashSet<u32> = reads.keys().copied().collect();
    while let Some(v) = struck.pop() {
        if set.remove(&v)
            && let Some(dependents) = readers.get(&v)
        {
            struck.extend(dependents.iter().copied().filter(|d| set.contains(d)));
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

/// One full adder: where its second operation and its `|` are, and what
/// they become.
struct Adder {
    /// `s2 = s1 + c` (or `d2 = d1 - c`), replaced by the word half.
    word_at: Position,
    word: Value,
    or_dest: Value,
    or_ty: IrType,
    op: CarryOp,
    args: [Operand; 3],
}

/// What the two partial carries say the adder is: the second operation,
/// its result, the operation and the operands.
type Found = (Position, Value, CarryOp, [Operand; 3]);

/// `c1 = s1 <u x|y` with `s1 = x + y`; `c2 = s2 <u s1|c` with `s2 = s1 + c`.
fn add_adder(func: &IrFunction, facts: &mut Facts, c1: Value, c2: Value) -> Option<Found> {
    let (s1_operand, bound1, ty1) = unsigned_less(func, facts, c1)?;
    let (s1, x, y, add1_ty) = binop(func, facts, &s1_operand, IrBinOp::Add)?;
    if bound1 != x && bound1 != y {
        return None;
    }
    let (s2_operand, bound2, ty2) = unsigned_less(func, facts, c2)?;
    let (s2, a, b, add2_ty) = binop(func, facts, &s2_operand, IrBinOp::Add)?;
    let s1_operand = Operand::Value(s1);
    let carry_in = if a == s1_operand {
        b
    } else if b == s1_operand {
        a
    } else {
        return None;
    };
    if bound2 != s1_operand && bound2 != carry_in {
        return None;
    }
    let op = word_op(add1_ty, false)?;
    if [add2_ty, ty1, ty2]
        .iter()
        .any(|ty| word_op(*ty, false) != Some(op))
    {
        return None;
    }
    let c = carry_of(func, facts, &carry_in)?;
    let word_at = (*facts.defs.get(s2.0 as usize)?)?;
    Some((word_at, s2, op, [x, y, c]))
}

/// `b1 = x <u y` with `d1 = x - y`; `b2 = d1 <u c` (or `d1 <u d2`) with
/// `d2 = d1 - c`. `or_at` is where the `|` is: the first spelling of `b2`
/// does not read the word, which must then be found, ahead of the `|` in
/// its block.
fn sub_adder(
    func: &IrFunction,
    facts: &mut Facts,
    b1: Value,
    b2: Value,
    or_at: Position,
) -> Option<Found> {
    let (x, y, ty1) = unsigned_less(func, facts, b1)?;
    let (d1_operand, bound, ty2) = unsigned_less(func, facts, b2)?;
    let (d1, sub_x, sub_y, sub1_ty) = binop(func, facts, &d1_operand, IrBinOp::Sub)?;
    if sub_x != x || sub_y != y {
        return None;
    }
    let op = word_op(sub1_ty, true)?;
    if word_op(ty1, true) != Some(op) || word_op(ty2, true) != Some(op) {
        return None;
    }
    let d1_operand = Operand::Value(d1);
    let is_word = |inst: &Instruction, carry_in: Option<&Operand>| match inst {
        Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ty,
            ..
        } => {
            *lhs == d1_operand
                && word_op(*ty, true) == Some(op)
                && carry_in.is_none_or(|c| rhs == c)
        }
        _ => false,
    };
    // `d1 <u d2`: the bound is the word.
    if let Operand::Value(d2) = &bound
        && let Some(inst @ Instruction::BinOp { rhs, .. }) = facts.def(func, *d2)
        && is_word(inst, None)
    {
        let c = carry_of(func, facts, rhs)?;
        let word_at = (*facts.defs.get(d2.0 as usize)?)?;
        return Some((word_at, *d2, op, [x, y, c]));
    }
    // `d1 <u c`: the bound is the carry in, and the word is the subtraction
    // of it from `d1`, looked for from the `|` upwards.
    let c = carry_of(func, facts, &bound)?;
    let (bi, or_index) = or_at;
    let (ii, word) = func.blocks[bi].instructions[..or_index]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(ii, inst)| {
            (is_word(inst, Some(&bound)))
                .then(|| inst.dest())
                .flatten()
                .map(|d| (ii, d))
        })?;
    Some(((bi, ii), word, op, [x, y, c]))
}

/// The adder whose carries the `|` at `or_at` combines, if it is one.
fn adder_at(func: &IrFunction, facts: &mut Facts, or_at: Position) -> Option<Adder> {
    let Instruction::BinOp {
        dest: or_dest,
        op: IrBinOp::Or,
        lhs: Operand::Value(first),
        rhs: Operand::Value(second),
        ty: or_ty,
    } = &func.blocks[or_at.0].instructions[or_at.1]
    else {
        return None;
    };
    if !is_integer(*or_ty) {
        return None;
    }
    let first = partial_carry(func, facts, *first)?;
    let second = partial_carry(func, facts, *second)?;
    [(first, second), (second, first)]
        .into_iter()
        .find_map(|(c1, c2)| {
            add_adder(func, facts, c1, c2).or_else(|| sub_adder(func, facts, c1, c2, or_at))
        })
        .map(|(word_at, word, op, args)| Adder {
            word_at,
            word,
            or_dest: *or_dest,
            or_ty: *or_ty,
            op,
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

/// Put `halves[0]` in place of the instruction at each position and
/// `halves[1]` behind it, and read `renamed` values by their new names
/// everywhere.
fn install(
    func: &mut IrFunction,
    mut pairs: Vec<(Position, [Instruction; 2])>,
    renamed: &FxHashMap<u32, Value>,
) {
    // From the back of each block, so that an insertion moves nothing that
    // is still to be placed.
    pairs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for ((bi, ii), [word, out]) in pairs {
        let block = &mut func.blocks[bi];
        let span = block.span_at(ii);
        block.instructions[ii] = word;
        block.insert_inst(ii + 1, out, span);
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
    // Most functions have no `|` of two values: nothing is computed for
    // them.
    let candidates: Vec<Position> = func
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(bi, block)| {
            block
                .instructions
                .iter()
                .enumerate()
                .filter(|(_, inst)| {
                    matches!(
                        inst,
                        Instruction::BinOp {
                            op: IrBinOp::Or,
                            lhs: Operand::Value(_),
                            rhs: Operand::Value(_),
                            ..
                        }
                    )
                })
                .map(move |(ii, _)| (bi, ii))
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut facts = Facts::of(func);
    let mut adders: Vec<Adder> = Vec::new();
    let mut claimed: FxHashSet<Position> = FxHashSet::default();
    for or_at in candidates {
        if let Some(adder) = adder_at(func, &mut facts, or_at)
            // Two `|` of the same adder's carries, spelled two ways: the
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
    // The carry out that each `|` (and each conversion of one) is.
    let mut carry: FxHashMap<u32, Value> = FxHashMap::default();
    // The ones that are read by that name: at 32 bits or fewer.
    let mut renamed: FxHashMap<u32, Value> = FxHashMap::default();
    let outs: Vec<Value> = adders
        .iter()
        .map(|adder| {
            let out = Value(next);
            next += 1;
            carry.insert(adder.or_dest.0, out);
            if narrow(adder.or_ty) {
                renamed.insert(adder.or_dest.0, out);
            }
            out
        })
        .collect();
    func.next_value_id = next;

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                // A 64-bit `|` is a conversion of the carry out.
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Or,
                    ty,
                    ..
                } if !narrow(*ty) && carry.contains_key(&dest.0) => {
                    *inst = Instruction::Cast {
                        dest: *dest,
                        src: Operand::Value(carry[&dest.0]),
                        from_ty: IrType::I32,
                        to_ty: *ty,
                    };
                }
                // A conversion of a carry out to 32 bits or fewer is the
                // carry out (`unsigned char c = _addcarry_u64(...)`), and a
                // wider one is one for the next link to look through.
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
                _ => {}
            }
        }
    }

    let count = adders.len();
    let pairs = adders
        .into_iter()
        .zip(outs)
        .map(|(mut adder, out)| {
            // The carry in of a link is the carry out of the one before.
            if let Operand::Value(c) = &adder.args[2]
                && let Some(&previous) = carry.get(&c.0)
            {
                adder.args[2] = Operand::Value(previous);
            }
            (
                adder.word_at,
                carry_halves(adder.op, adder.word, out, &adder.args),
            )
        })
        .collect();
    install(func, pairs, &renamed);
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
    let mut pairs: Vec<(Position, [Instruction; 2])> = Vec::new();
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
        pairs.push((at, carry_halves(op, word, out, &args)));
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

    /// `d1 <u c` does not read the word: one that stands behind the `|`
    /// cannot take the carry out with it.
    #[test]
    fn a_word_behind_the_or_is_left_alone() {
        let mut insts = header_subtractor(cmp(15, IrCmpOp::Ult, v(10), v(12), U64));
        let word = insts.remove(6);
        insts.push(word);
        insts.push(bin(20, IrBinOp::Xor, v(13), v(16), U64));
        let mut func = function(insts, v(20));
        assert_eq!(run(&mut func, true), 0);
    }

    /// A 64-bit `|` of converted comparisons (`carry = (s < a) | (t < s)`
    /// in `unsigned long long`) is a conversion of the carry out.
    #[test]
    fn a_wide_or_becomes_a_conversion() {
        let insts = vec![
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
            bin(16, IrBinOp::Or, v(12), v(15), U64),
            bin(17, IrBinOp::Xor, v(16), v(13), U64),
        ];
        let mut func = function(insts, v(17));
        assert_eq!(run(&mut func, true), 1);
        let found = halves(&func);
        assert_eq!(found[0].3, vec![v(0), v(1), v(3)]);
        let out = found[1].2;
        assert!(func.blocks[0].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Cast { dest: Value(16), src: Operand::Value(s), to_ty: IrType::U64, .. }
                if *s == out
        )));
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
        let set = booleans(&func);
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
