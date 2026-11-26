//! Tests of `limb_unroll`. The loops are run: every shape the pass takes,
//! as it was and split, over the trip counts round a trip of the main loop
//! and the values at which a counter wraps; and the two runs have to agree
//! load by load and store by store.

use super::*;
use crate::common::types::AddressSpace;
use crate::passes::verify;

const U64: IrType = IrType::U64;
const I64: IrType = IrType::I64;
const PTR: IrType = IrType::Ptr;

// ── A function, by hand ──────────────────────────────────────────────────

fn v(id: u32) -> Operand {
    Operand::Value(Value(id))
}

fn typed(value: i64, ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(value, ty))
}

fn wide(value: i64) -> Operand {
    Operand::Const(IrConst::I64(value))
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

fn phi(dest: u32, ty: IrType, incoming: Vec<(Operand, u32)>) -> Instruction {
    Instruction::Phi {
        dest: Value(dest),
        ty,
        incoming: incoming
            .into_iter()
            .map(|(operand, from)| (operand, BlockId(from)))
            .collect(),
    }
}

fn param(dest: u32, param_idx: usize, ty: IrType) -> Instruction {
    Instruction::ParamRef {
        dest: Value(dest),
        param_idx,
        ty,
    }
}

fn load(dest: u32, ptr: u32) -> Instruction {
    Instruction::Load {
        dest: Value(dest),
        ptr: Value(ptr),
        ty: U64,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

fn store(val: Operand, ptr: u32) -> Instruction {
    Instruction::Store {
        val,
        ptr: Value(ptr),
        ty: U64,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

fn address(dest: u32, base: u32, offset: Operand) -> Instruction {
    Instruction::GetElementPtr {
        dest: Value(dest),
        base: Value(base),
        offset,
        ty: PTR,
    }
}

/// How the loop asks whether to go on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Test {
    /// `counter OP limit` goes on.
    Goes(IrCmpOp),
    /// `limit OP counter` goes on.
    Swapped(IrCmpOp),
    /// `counter OP limit` leaves.
    Leaves(IrCmpOp),
    /// The counter itself: anything but zero goes on (`while (n)`).
    Itself,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Limit {
    /// The fifth argument.
    Argument,
    Constant(i64),
}

/// A limb loop: `r[] = a[] + b[]` with the carry round the back edge.
///
/// Values: 1 to 5 the arguments (`r`, `a`, `b`, the counter's first value,
/// the limit); 10 the counter, 11 the carry, 12 to 14 the pointers, 9 and
/// 15 to 17 the phis of `more`; 20 to 22 the addresses or the stepped
/// pointers; 30 to 38 the adder, 35 its sum and 38 its carry out; 40 the
/// stepped counter, 41 the test, 46 the stepped second counter of `more`,
/// 47 the counter narrowed for the test.
/// Blocks: 0 the way in, 1 the header, 2 the latch of a loop tested at the
/// top, 3 behind the loop.
#[derive(Clone, Copy, Debug)]
struct Limbs {
    /// The counter's type.
    ty: IrType,
    /// `do ... while`.
    bottom: bool,
    /// `*p++`; otherwise `p[i]`.
    pointers: bool,
    /// The limbs are read through the stepped pointers (`*++p`), so the
    /// steps are read by more than their phis.
    stepped_first: bool,
    up: bool,
    test: Test,
    /// Tested at the bottom, the test reads the stepped counter.
    tests_next: bool,
    limit: Limit,
    /// Four more phis, none of them work between two limbs: one that
    /// receives a constant, one that receives the counter of the iteration
    /// before, the last limb kept (`last = t`), and a second counter that
    /// goes down by five. And behind the loop everything of it is read;
    /// otherwise the carry alone.
    more: bool,
    /// The test reads the counter narrowed to this type (value 47), and
    /// the limit is of it: `for (int i = 0; i < n; i++)` with the counter
    /// widened to index with. `ty` is the wide type then.
    narrowed: Option<IrType>,
    /// Tested at the top, the counter's step stands in the header in front
    /// of the test: `while (n-- > 0)`.
    step_in_header: bool,
}

impl Limbs {
    /// GMP's `mpn_add_n`, as the optimizer leaves it.
    fn gmp() -> Self {
        Limbs {
            ty: I64,
            bottom: true,
            pointers: true,
            stepped_first: false,
            up: false,
            test: Test::Goes(IrCmpOp::Ne),
            tests_next: false,
            limit: Limit::Constant(1),
            more: false,
            narrowed: None,
            step_in_header: false,
        }
    }

    /// mini-gmp's `mpn_add_n`.
    fn mini() -> Self {
        Limbs {
            ty: I64,
            bottom: false,
            pointers: false,
            stepped_first: false,
            up: true,
            test: Test::Goes(IrCmpOp::Slt),
            tests_next: false,
            limit: Limit::Argument,
            more: false,
            narrowed: None,
            step_in_header: false,
        }
    }

    fn build(&self) -> IrFunction {
        use IrBinOp::{Add, Or, Shl, Sub, Xor};
        let ty = self.ty;
        let test_ty = self.narrowed.unwrap_or(ty);
        let latch = if self.bottom { 1 } else { 2 };
        let limit = match self.limit {
            Limit::Argument => v(5),
            Limit::Constant(value) => typed(value, test_ty),
        };
        let mut phis = vec![
            phi(10, ty, vec![(v(4), 0), (v(40), latch)]),
            phi(11, U64, vec![(wide(0), 0), (v(38), latch)]),
        ];
        let mut work = Vec::new();
        let (a, b, r) = if self.pointers {
            phis.push(phi(12, PTR, vec![(v(1), 0), (v(22), latch)]));
            phis.push(phi(13, PTR, vec![(v(2), 0), (v(20), latch)]));
            phis.push(phi(14, PTR, vec![(v(3), 0), (v(21), latch)]));
            work.push(bin(20, Add, v(13), wide(8), I64));
            work.push(bin(21, Add, v(14), wide(8), I64));
            work.push(bin(22, Add, v(12), wide(8), I64));
            if self.stepped_first {
                (20, 21, 22)
            } else {
                (13, 14, 12)
            }
        } else {
            let index = if ty.size() == 8 {
                10
            } else {
                work.push(cast(18, 10, ty, I64));
                18
            };
            work.push(bin(19, Shl, v(index), wide(3), I64));
            work.push(address(20, 2, v(19)));
            work.push(address(21, 3, v(19)));
            work.push(address(22, 1, v(19)));
            (20, 21, 22)
        };
        work.extend([
            load(30, a),
            load(31, b),
            bin(32, Add, v(30), v(31), U64),
            cmp(33, IrCmpOp::Ult, v(32), v(30), U64),
            cast(34, 33, IrType::I32, U64),
            bin(35, Add, v(32), v(11), U64),
            cmp(36, IrCmpOp::Ult, v(35), v(32), U64),
            cast(37, 36, IrType::I32, U64),
            bin(38, Or, v(34), v(37), U64),
            store(v(35), r),
        ]);
        if self.more {
            phis.push(phi(15, U64, vec![(wide(7), 0), (wide(1), latch)]));
            phis.push(phi(16, ty, vec![(typed(0, ty), 0), (v(10), latch)]));
            phis.push(phi(17, U64, vec![(wide(0), 0), (v(35), latch)]));
            phis.push(phi(9, U64, vec![(wide(1000), 0), (v(46), latch)]));
            work.push(bin(46, Sub, v(9), wide(5), U64));
        }
        let step = bin(40, if self.up { Add } else { Sub }, v(10), typed(1, ty), ty);
        // What the header of a loop tested at the top holds in front of
        // its test.
        let mut asked = Vec::new();
        if self.step_in_header && !self.bottom {
            asked.push(step);
        } else {
            work.push(step);
        }
        let tested = if self.bottom && self.tests_next {
            40
        } else {
            10
        };
        let tested = match self.narrowed {
            Some(narrow) => {
                let narrowed = cast(47, tested, ty, narrow);
                if self.bottom {
                    work.push(narrowed);
                } else {
                    asked.push(narrowed);
                }
                v(47)
            }
            None => v(tested),
        };
        let (cond, test, goes_on_true) = match self.test {
            Test::Goes(op) => (v(41), Some(cmp(41, op, tested, limit, test_ty)), true),
            Test::Swapped(op) => (v(41), Some(cmp(41, op, limit, tested, test_ty)), true),
            Test::Leaves(op) => (v(41), Some(cmp(41, op, tested, limit, test_ty)), false),
            Test::Itself => (tested, None, true),
        };
        let leave_or_stay = |stay: u32| {
            let (true_label, false_label) = if goes_on_true { (stay, 3) } else { (3, stay) };
            Terminator::CondBranch {
                cond,
                true_label: BlockId(true_label),
                false_label: BlockId(false_label),
            }
        };

        // Behind the loop everything of it that can be read is read. At
        // the top that is the header's phis; at the bottom the iteration's
        // own values as well (the stepped pointers only where the limbs
        // are read through them, so that the plain `*p++` keeps steps
        // nothing but their phis read).
        let mut read: Vec<(u32, IrType)> = vec![(if self.bottom { 38 } else { 11 }, U64)];
        if self.step_in_header && !self.bottom {
            // `n` behind `while (n-- > 0)`: one step past the limit.
            read.push((40, ty));
        }
        if self.more {
            read.extend([
                (11, U64),
                (10, ty),
                (15, U64),
                (16, ty),
                (17, U64),
                (9, U64),
            ]);
            if self.pointers {
                read.push((12, PTR));
            }
            if self.bottom {
                read.extend([(40, ty), (35, U64), (46, U64)]);
                if self.pointers && self.stepped_first {
                    read.push((22, I64));
                }
            }
        }
        let mut behind = Vec::new();
        let mut all: Option<u32> = None;
        for (at, &(value, value_ty)) in read.iter().enumerate() {
            let at = 60 + 4 * at as u32;
            let value = if value_ty.size() == 8 {
                value
            } else {
                behind.push(cast(at, value, value_ty, U64));
                at
            };
            all = Some(match all {
                None => value,
                Some(before) => {
                    behind.push(bin(at + 1, Shl, v(before), wide(7), U64));
                    behind.push(bin(at + 2, Xor, v(at + 1), v(value), U64));
                    at + 2
                }
            });
        }
        let result = v(all.expect("something is read behind the loop"));

        let mut func = IrFunction::new("limbs".into(), U64, vec![], false);
        let way_in = vec![
            param(1, 0, PTR),
            param(2, 1, PTR),
            param(3, 2, PTR),
            param(4, 3, ty),
            param(5, 4, test_ty),
        ];
        func.blocks
            .push(block(BlockId(0), way_in, Terminator::Branch(BlockId(1))));
        if self.bottom {
            let mut body = phis;
            body.append(&mut work);
            body.extend(test);
            func.blocks.push(block(BlockId(1), body, leave_or_stay(1)));
        } else {
            let mut header = phis;
            header.append(&mut asked);
            header.extend(test);
            func.blocks
                .push(block(BlockId(1), header, leave_or_stay(2)));
            func.blocks
                .push(block(BlockId(2), work, Terminator::Branch(BlockId(1))));
        }
        func.blocks
            .push(block(BlockId(3), behind, Terminator::Return(Some(result))));
        func.next_label = 4;
        func.next_value_id = func.sound_next_value_id();
        func
    }
}

// ── A machine ────────────────────────────────────────────────────────────

/// The loads and stores a run stops after: enough for three trips of the
/// main loop and a remainder, and an end to a loop of 2^64 iterations.
const FUEL: usize = 96;

/// What a run did: its loads and stores in order (store?, address, value),
/// how often each block was entered, and what it returned; `None` where it
/// was stopped.
#[derive(Debug)]
struct Ran {
    memory: Vec<(bool, u64, u64)>,
    entered: FxHashMap<u32, usize>,
    result: Option<u64>,
}

fn width(ty: IrType) -> u32 {
    match ty.size() {
        1 => 8,
        2 => 16,
        4 => 32,
        _ => 64,
    }
}

fn cut(value: u64, ty: IrType) -> u64 {
    value & (u64::MAX >> (64 - width(ty)))
}

fn extended(value: u64, ty: IrType) -> i64 {
    signed(value as i64, width(ty))
}

/// What a word that was never stored holds: something of its address, and
/// in every third word of `b` the complement of the word of `a` beside it,
/// so that a carry in is a carry out.
fn never_stored(at: u64) -> u64 {
    let mixed = |at: u64| {
        (at ^ 0x9e37_79b9_7f4a_7c15)
            .wrapping_mul(0xbf58_476d_1ce4_e5b9)
            .rotate_left(29)
    };
    if at & 0xf_0000 == 0x3_0000 && (at >> 3) % 3 != 1 {
        !mixed(at - 0x1_0000)
    } else {
        mixed(at)
    }
}

/// Runs `func` on `arguments`. Arithmetic is in the width of its type,
/// comparisons in theirs, and the phis of a block are read all at once.
fn ran(func: &IrFunction, arguments: &[u64]) -> Ran {
    let mut values = vec![0u64; func.sound_next_value_id() as usize];
    let mut stored: FxHashMap<u64, u64> = FxHashMap::default();
    let mut out = Ran {
        memory: Vec::new(),
        entered: FxHashMap::default(),
        result: None,
    };
    let read = |operand: &Operand, values: &[u64]| match operand {
        Operand::Const(c) => c.to_i64().expect("an integer") as u64,
        Operand::Value(value) => values[value.0 as usize],
    };
    let mut at = 0;
    let mut from = BlockId(u32::MAX);
    loop {
        let block = &func.blocks[at];
        *out.entered.entry(block.label.0).or_default() += 1;
        let arriving: Vec<(u32, u64)> = block
            .instructions
            .iter()
            .map_while(|inst| match inst {
                Instruction::Phi { dest, ty, incoming } => {
                    let (operand, _) = incoming
                        .iter()
                        .find(|(_, label)| *label == from)
                        .unwrap_or_else(|| panic!("{dest:?} has nothing for {from:?}"));
                    Some((dest.0, cut(read(operand, &values), *ty)))
                }
                _ => None,
            })
            .collect();
        for (dest, value) in arriving {
            values[dest as usize] = value;
        }
        for inst in &block.instructions {
            let (dest, value) = match inst {
                Instruction::Phi { .. } => continue,
                Instruction::ParamRef {
                    dest,
                    param_idx,
                    ty,
                } => (dest, cut(arguments[*param_idx], *ty)),
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (l, r) = (read(lhs, &values), read(rhs, &values));
                    let value = match op {
                        IrBinOp::Add => l.wrapping_add(r),
                        IrBinOp::Sub => l.wrapping_sub(r),
                        IrBinOp::Mul => l.wrapping_mul(r),
                        IrBinOp::And => l & r,
                        IrBinOp::Or => l | r,
                        IrBinOp::Xor => l ^ r,
                        IrBinOp::Shl => l << (r % u64::from(width(*ty))),
                        IrBinOp::LShr => cut(l, *ty) >> (r % u64::from(width(*ty))),
                        other => panic!("the machine has no {other:?}"),
                    };
                    (dest, cut(value, *ty))
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (l, r) = (cut(read(lhs, &values), *ty), cut(read(rhs, &values), *ty));
                    let (sl, sr) = (extended(l, *ty), extended(r, *ty));
                    let holds = match op {
                        IrCmpOp::Eq => l == r,
                        IrCmpOp::Ne => l != r,
                        IrCmpOp::Slt => sl < sr,
                        IrCmpOp::Sle => sl <= sr,
                        IrCmpOp::Sgt => sl > sr,
                        IrCmpOp::Sge => sl >= sr,
                        IrCmpOp::Ult => l < r,
                        IrCmpOp::Ule => l <= r,
                        IrCmpOp::Ugt => l > r,
                        IrCmpOp::Uge => l >= r,
                    };
                    (dest, u64::from(holds))
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let value = cut(read(src, &values), *from_ty);
                    let value = if from_ty.is_signed() {
                        extended(value, *from_ty) as u64
                    } else {
                        value
                    };
                    (dest, cut(value, *to_ty))
                }
                Instruction::Copy { dest, src } => (dest, read(src, &values)),
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => (
                    dest,
                    values[base.0 as usize].wrapping_add(read(offset, &values)),
                ),
                Instruction::Load { dest, ptr, ty, .. } => {
                    let at = values[ptr.0 as usize];
                    let value = stored.get(&at).copied().unwrap_or_else(|| never_stored(at));
                    out.memory.push((false, at, value));
                    (dest, cut(value, *ty))
                }
                Instruction::Store { val, ptr, ty, .. } => {
                    let at = values[ptr.0 as usize];
                    let value = cut(read(val, &values), *ty);
                    stored.insert(at, value);
                    out.memory.push((true, at, value));
                    if out.memory.len() >= FUEL {
                        return out;
                    }
                    continue;
                }
                other => panic!("the machine has no {other:?}"),
            };
            values[dest.0 as usize] = value;
        }
        let to = match &block.terminator {
            Terminator::Return(result) => {
                out.result = Some(result.as_ref().map_or(0, |result| read(result, &values)));
                return out;
            }
            Terminator::Branch(to) => *to,
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                if read(cond, &values) != 0 {
                    *true_label
                } else {
                    *false_label
                }
            }
            other => panic!("the machine has no {other:?}"),
        };
        from = block.label;
        at = func
            .blocks
            .iter()
            .position(|block| block.label == to)
            .unwrap_or_else(|| panic!("no block {to:?}"));
    }
}

// ── What is asked of a split ─────────────────────────────────────────────

/// The label of the main loop of a split function: the block of four
/// stores.
fn main_loop(func: &IrFunction) -> u32 {
    let mut found = func.blocks.iter().filter(|block| {
        block
            .instructions
            .iter()
            .filter(|inst| matches!(inst, Instruction::Store { .. }))
            .count()
            == COPIES as usize
    });
    let main = found.next().expect("a block of four stores");
    assert!(found.next().is_none(), "one main loop");
    main.label.0
}

fn well_formed(func: &IrFunction, what: &dyn std::fmt::Debug) {
    let mut violations = Vec::new();
    verify::verify_function(func, "limb_unroll", &mut violations);
    assert!(violations.is_empty(), "{what:?}: {violations:#?}");
}

/// Splits the loop of `limbs`, or finds it refused, as `splits` says; and
/// runs it before and after on every one of `counters` (first value,
/// limit), with the three numbers apart, in place, and with the result one
/// limb above an operand, where every store is the next iteration's load.
fn compared(limbs: &Limbs, splits: bool, counters: &[(u64, u64)]) {
    compared_built(limbs.build(), limbs, splits, counters);
}

/// `compared` for a function that is built already; `limbs` says what it
/// is in a failure.
fn compared_built(
    before: IrFunction,
    limbs: &dyn std::fmt::Debug,
    splits: bool,
    counters: &[(u64, u64)],
) {
    well_formed(&before, limbs);
    let mut after = before.clone();
    assert_eq!(run(&mut after), usize::from(splits), "{limbs:?}");
    well_formed(&after, limbs);
    if !splits {
        assert_eq!(format!("{before:?}"), format!("{after:?}"), "{limbs:?}");
        return;
    }
    let main = main_loop(&after);
    let mut again = after.clone();
    assert_eq!(run(&mut again), 0, "{limbs:?}: split twice");

    let placed = [
        (0x1_0000, 0x2_0000, 0x3_0000),
        (0x2_0000, 0x2_0000, 0x3_0000),
        (0x2_0008, 0x2_0000, 0x3_0000),
        (0x3_0008, 0x2_0000, 0x3_0000),
    ];
    let mut trips_seen = 0;
    for (nth, &(first, limit)) in counters.iter().enumerate() {
        // Every placement for the first counts, then one in turn.
        let placed = if nth < 14 {
            &placed[..]
        } else {
            &placed[nth % 4..=nth % 4]
        };
        for &(r, a, b) in placed {
            let arguments = [r, a, b, first, limit];
            let (was, is) = (ran(&before, &arguments), ran(&after, &arguments));
            let what = format!("{limbs:?} from {first:#x} to {limit:#x}, r = {r:#x}");
            assert_eq!(was.memory, is.memory, "{what}");
            assert_eq!(was.result, is.result, "{what}");
            // A count that is known, and of four or more: the main loop
            // ran every whole trip of it.
            let trips = is.entered.get(&main).copied().unwrap_or(0);
            if was.result.is_some() {
                let limbs_done = was.memory.len() / 3;
                assert!(trips <= limbs_done / 4, "{what}: {trips} trips");
                assert!(
                    trips == 0 || trips == limbs_done / 4,
                    "{what}: {trips} trips of {limbs_done} limbs"
                );
            }
            trips_seen += trips;
        }
    }
    assert!(trips_seen > 0, "{limbs:?}: the main loop never ran");
}

/// First values and limits for a counter of type `ty`: every count from
/// nothing to three trips and a remainder, from a first value in the
/// middle, one below zero and those next to where the type wraps, signed
/// and unsigned; and the limit on the far side.
fn counters(ty: IrType, up: bool) -> Vec<(u64, u64)> {
    let bits = width(ty);
    let most = (u64::MAX >> (64 - bits)) >> 1;
    let firsts = [
        5,
        0,
        cut(u64::MAX - 2, ty),
        most - 2,
        most + 2,
        cut(u64::MAX - 20, ty),
        most + 20,
    ];
    let mut all = Vec::new();
    for first in firsts {
        for count in (0..=13).chain([cut(u64::MAX, ty), cut(u64::MAX - 4, ty), most, most + 1]) {
            let limit = if up {
                first.wrapping_add(count)
            } else {
                first.wrapping_sub(count)
            };
            all.push((first, cut(limit, ty)));
        }
    }
    all
}

/// The tests a loop that steps `up` (or down) is taken with, against a
/// limit that is an argument.
fn tests(up: bool) -> Vec<Test> {
    use IrCmpOp::{Eq, Ne, Sge, Sgt, Sle, Slt, Uge, Ugt, Ule, Ult};
    let mut tests = vec![Test::Goes(Ne), Test::Swapped(Ne), Test::Leaves(Eq)];
    tests.extend(if up {
        [
            Test::Goes(Slt),
            Test::Goes(Ult),
            Test::Swapped(Sgt),
            Test::Swapped(Ugt),
            Test::Leaves(Sge),
            Test::Leaves(Uge),
        ]
    } else {
        [
            Test::Goes(Sgt),
            Test::Goes(Ugt),
            Test::Swapped(Slt),
            Test::Swapped(Ult),
            Test::Leaves(Sle),
            Test::Leaves(Ule),
        ]
    });
    tests
}

const TYPES: [IrType; 4] = [IrType::I32, IrType::U32, IrType::I64, IrType::U64];

// ── The loops of the libraries ───────────────────────────────────────────

#[test]
fn gmp_and_mini_gmp_are_split_and_run_as_before() {
    let gmp = Limbs::gmp();
    let counts: Vec<(u64, u64)> = (0..=13)
        .chain([u64::MAX, 1 << 63])
        .map(|n| (n, 0))
        .collect();
    compared(&gmp, true, &counts);
    compared(&Limbs::mini(), true, &counters(I64, true));

    // GMP's loop, split: the guard, the way into the main loop, the main
    // loop, what is behind it, and the join in front of the exit.
    let mut func = gmp.build();
    assert_eq!(run(&mut func), 1);
    assert_eq!(func.blocks.len(), 3 + 5);
    let main = main_loop(&func);
    let main = func
        .blocks
        .iter()
        .find(|block| block.label.0 == main)
        .expect("the main loop");
    // Its pointers are stepped as addresses off the trip's own pointer, by
    // one, two and three limbs for the copies and by four for the trip.
    let mut steps: Vec<i64> = main
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            Instruction::GetElementPtr {
                offset: Operand::Const(offset),
                ..
            } => offset.to_i64(),
            _ => None,
        })
        .collect();
    steps.sort_unstable();
    assert_eq!(steps, [8, 8, 8, 16, 16, 16, 24, 24, 24, 32, 32, 32]);
    // And the last of them stand behind the last store: nothing but the
    // phis reads them.
    let last_store = main
        .instructions
        .iter()
        .rposition(|inst| matches!(inst, Instruction::Store { .. }))
        .expect("a store");
    let behind: Vec<i64> = main.instructions[last_store..]
        .iter()
        .filter_map(|inst| match inst {
            Instruction::GetElementPtr {
                offset: Operand::Const(offset),
                ..
            } => offset.to_i64(),
            _ => None,
        })
        .collect();
    assert_eq!(behind, [32, 32, 32]);
    // The trips are counted down by one, last of all.
    let [.., left, again] = &main.instructions[..] else {
        panic!("a short main loop");
    };
    assert!(
        matches!(
            (left, again),
            (
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    rhs: Operand::Const(by),
                    ..
                },
                Instruction::Cmp {
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(tested),
                    rhs: Operand::Const(nothing),
                    ..
                },
            ) if dest == tested && by.to_i64() == Some(1) && nothing.to_i64() == Some(0)
        ),
        "{left:?} {again:?}"
    );
}

// ── Every shape ──────────────────────────────────────────────────────────

/// The sweep of one counter type: tested at the top and at the bottom,
/// through pointers and through an index, up and down, with every test
/// that names the limit, and the test on the counter before or behind its
/// step.
fn every_shape(ty: IrType) {
    for bottom in [false, true] {
        for pointers in [false, true] {
            for up in [true, false] {
                for test in tests(up) {
                    for tests_next in [false, true] {
                        if tests_next && !bottom {
                            continue;
                        }
                        for more in [false, true] {
                            let limbs = Limbs {
                                ty,
                                bottom,
                                pointers,
                                stepped_first: pointers && more == up,
                                up,
                                test,
                                tests_next,
                                limit: Limit::Argument,
                                more,
                                narrowed: None,
                                step_in_header: false,
                            };
                            compared(&limbs, true, &counters(ty, up));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn every_shape_with_a_32_bit_signed_counter() {
    every_shape(IrType::I32);
}

#[test]
fn every_shape_with_a_32_bit_unsigned_counter() {
    every_shape(IrType::U32);
}

#[test]
fn every_shape_with_a_64_bit_signed_counter() {
    every_shape(IrType::I64);
}

#[test]
fn every_shape_with_a_64_bit_unsigned_counter() {
    every_shape(IrType::U64);
}

/// `while (n)`, `do ... while (--n)` and `do ... while (n--)`: the counter
/// is its own test.
#[test]
fn a_counter_that_is_its_own_test() {
    for ty in TYPES {
        let counts: Vec<(u64, u64)> = (0..=13)
            .chain([cut(u64::MAX, ty), cut(u64::MAX - 5, ty)])
            .map(|n| (n, 77))
            .collect();
        for (bottom, tests_next) in [(false, false), (true, true), (true, false)] {
            for pointers in [false, true] {
                let limbs = Limbs {
                    ty,
                    bottom,
                    pointers,
                    stepped_first: false,
                    up: false,
                    test: Test::Itself,
                    tests_next,
                    limit: Limit::Argument,
                    more: pointers,
                    narrowed: None,
                    step_in_header: false,
                };
                compared(&limbs, true, &counts);
            }
        }
    }
}

/// `counters` of the narrow type under a wide first value: what is above
/// the narrow type is the wide counter's own business.
fn narrowed_counters(narrow: IrType, up: bool) -> Vec<(u64, u64)> {
    counters(narrow, up)
        .into_iter()
        .enumerate()
        .map(|(nth, (first, limit))| {
            let above: u64 = [0, 1, 0x7fff_ffff, 0xffff_ffff][nth % 4];
            (first | above << 32, limit)
        })
        .collect()
}

/// `for (int i = 0; i < n; i++) r[i] = ...`: the counter was widened to
/// index with, and the test reads it narrowed. The count is the narrow
/// type's -- what the test sees of the counter steps by one there -- and
/// the wide counter is carried through the copies as it is.
#[test]
fn a_counter_the_test_reads_narrowed() {
    for narrow in [IrType::I32, IrType::U32] {
        for bottom in [false, true] {
            for up in [true, false] {
                for test in tests(up) {
                    for tests_next in [false, true] {
                        if tests_next && !bottom {
                            continue;
                        }
                        for pointers in [false, true] {
                            let limbs = Limbs {
                                ty: if up { I64 } else { U64 },
                                bottom,
                                pointers,
                                stepped_first: false,
                                up,
                                test,
                                tests_next,
                                limit: Limit::Argument,
                                more: pointers == up,
                                narrowed: Some(narrow),
                                step_in_header: false,
                            };
                            compared(&limbs, true, &narrowed_counters(narrow, up));
                        }
                    }
                }
            }
        }
    }
    // `while ((int)n)`: the narrowed counter is its own test.
    for bottom in [false, true] {
        let limbs = Limbs {
            ty: I64,
            bottom,
            up: false,
            test: Test::Itself,
            tests_next: bottom,
            limit: Limit::Argument,
            narrowed: Some(IrType::U32),
            ..Limbs::mini()
        };
        let counts: Vec<(u64, u64)> = (0..=13)
            .chain([0xffff_ffff, 0xffff_fffa])
            .map(|n| (n | 5 << 32, 77))
            .collect();
        compared(&limbs, true, &counts);
    }
    // Against a constant, inclusive: the strict test against the constant
    // beside it, in the narrow type.
    let limbs = Limbs {
        ty: I64,
        test: Test::Goes(IrCmpOp::Sle),
        limit: Limit::Constant(9),
        narrowed: Some(IrType::I32),
        ..Limbs::mini()
    };
    compared(
        &limbs,
        true,
        &[
            (0, 0),
            (3, 0),
            (6, 0),
            (7, 0),
            (1 << 32, 0),
            (9, 0),
            (10, 0),
        ],
    );

    // The narrowed counter is the header's: an iteration that reads it, or
    // a phi that receives it, has no copy of it.
    let mut func = Limbs {
        narrowed: Some(IrType::I32),
        ..Limbs::mini()
    }
    .build();
    let (work, start) = work_of(&mut func, false);
    work.insert(start, cast(150, 47, IrType::I32, U64));
    left_alone(func, "the narrowed counter read by the work");
    let mut func = Limbs {
        narrowed: Some(IrType::I32),
        ..Limbs::mini()
    }
    .build();
    func.blocks[1].instructions.insert(
        2,
        phi(
            150,
            IrType::I32,
            vec![(typed(0, IrType::I32), 0), (v(47), 2)],
        ),
    );
    left_alone(func, "the narrowed counter handed round the loop");
    // Narrowed, but not what the test reads.
    let mut func = Limbs::mini().build();
    func.blocks[1]
        .instructions
        .insert(2, cast(47, 10, I64, IrType::I32));
    left_alone(func, "a narrowed counter the test does not read");
    // To 16 bits: no counter this pass counts in.
    let mut func = Limbs {
        narrowed: Some(IrType::I32),
        ..Limbs::mini()
    }
    .build();
    for inst in &mut func.blocks[1].instructions {
        match inst {
            Instruction::Cast { to_ty, .. } => *to_ty = IrType::I16,
            Instruction::Cmp { ty, .. } => *ty = IrType::I16,
            _ => {}
        }
    }
    left_alone(func, "a counter narrowed to 16 bits");
}

/// `while (n-- > 0)`: the counter's step stands in the header in front of
/// the test, and runs once more than the work does. It is the first thing
/// of an iteration; the one more is the header's own, of the loop as it
/// was, and what is read of it behind the loop is that.
#[test]
fn a_step_in_the_header_is_the_first_thing_of_an_iteration() {
    for ty in TYPES {
        for up in [false, true] {
            for test in tests(up) {
                for pointers in [false, true] {
                    for more in [false, true] {
                        let limbs = Limbs {
                            ty,
                            bottom: false,
                            pointers,
                            stepped_first: false,
                            up,
                            test,
                            tests_next: false,
                            limit: Limit::Argument,
                            more,
                            narrowed: None,
                            step_in_header: true,
                        };
                        compared(&limbs, true, &counters(ty, up));
                    }
                }
            }
        }
        // `while (n--)`.
        let limbs = Limbs {
            ty,
            up: false,
            test: Test::Itself,
            limit: Limit::Argument,
            step_in_header: true,
            ..Limbs::mini()
        };
        let counts: Vec<(u64, u64)> = (0..=13)
            .chain([cut(u64::MAX, ty), cut(u64::MAX - 5, ty)])
            .map(|n| (n, 77))
            .collect();
        compared(&limbs, true, &counts);
    }
    // And with the counter narrowed for the test: `while ((int)n-- > 0)`.
    for more in [false, true] {
        let limbs = Limbs {
            ty: I64,
            up: false,
            test: Test::Goes(IrCmpOp::Sgt),
            limit: Limit::Constant(0),
            more,
            narrowed: Some(IrType::I32),
            step_in_header: true,
            ..Limbs::mini()
        };
        compared(&limbs, true, &narrowed_counters(IrType::I32, false));
    }

    let stepping = Limbs {
        up: false,
        test: Test::Goes(IrCmpOp::Sgt),
        limit: Limit::Constant(0),
        step_in_header: true,
        ..Limbs::mini()
    };
    // The header is `[counter, carry, step, test]`.
    let header_of = |func: &IrFunction| -> Vec<Instruction> { func.blocks[1].instructions.clone() };
    assert!(
        matches!(
            &header_of(&stepping.build())[2..],
            [Instruction::BinOp { .. }, Instruction::Cmp { .. }]
        ),
        "the header this test is about"
    );
    // `while (--n > 0)`: the test reads the stepped counter.
    let mut func = stepping.build();
    if let Instruction::Cmp { lhs, .. } = &mut func.blocks[1].instructions[3] {
        *lhs = v(40);
    }
    left_alone(func, "a test of the counter stepped in the header");
    // `while (--n)`.
    let mut func = stepping.build();
    func.blocks[1].instructions.truncate(3);
    func.blocks[1].terminator = Terminator::CondBranch {
        cond: v(40),
        true_label: BlockId(2),
        false_label: BlockId(3),
    };
    left_alone(func, "the counter stepped in the header as its own test");
    // The step of a phi that is not the counter.
    let pointers = Limbs {
        pointers: true,
        ..Limbs::mini()
    };
    let mut func = pointers.build();
    let (work, start) = work_of(&mut func, false);
    let step = work.remove(start);
    assert!(matches!(
        step,
        Instruction::BinOp {
            dest: Value(20),
            ..
        }
    ));
    func.blocks[1].instructions.insert(5, step);
    left_alone(func, "a pointer stepped in the header");
    // Two steps.
    let mut func = Limbs {
        pointers: true,
        ..stepping
    }
    .build();
    let (work, start) = work_of(&mut func, false);
    let step = work.remove(start);
    func.blocks[1].instructions.insert(5, step);
    left_alone(func, "two steps in the header");
}

/// `<=` and `>=` against a constant are the strict test against the
/// constant beside it; against the last constant of the type, or against
/// anything that is not a constant, the loop is left alone.
#[test]
fn an_inclusive_test_is_taken_against_a_constant_with_a_neighbour() {
    use IrCmpOp::{Sge, Sgt, Sle, Slt, Uge, Ugt, Ule, Ult};
    for ty in TYPES {
        let bits = width(ty);
        let most = (i64::MAX as u64 >> (64 - bits)) as i64;
        let least = -most - 1;
        // (up, test, limit, splits)
        let cases = [
            (true, Test::Goes(Sle), 7, true),
            (true, Test::Goes(Sle), -1, true),
            (true, Test::Goes(Sle), most - 1, true),
            (true, Test::Goes(Sle), most, false),
            (true, Test::Goes(Ule), 7, true),
            (true, Test::Goes(Ule), most, true),
            (true, Test::Goes(Ule), -2, true),
            (true, Test::Goes(Ule), -1, false),
            (true, Test::Leaves(Sgt), 7, true),
            (true, Test::Leaves(Sgt), most, false),
            (true, Test::Leaves(Ugt), 40, true),
            (true, Test::Leaves(Ugt), -1, false),
            (true, Test::Swapped(Sge), 7, true),
            (true, Test::Swapped(Uge), -1, false),
            (false, Test::Goes(Sge), 7, true),
            (false, Test::Goes(Sge), 0, true),
            (false, Test::Goes(Sge), least + 1, true),
            (false, Test::Goes(Sge), least, false),
            (false, Test::Goes(Uge), 7, true),
            (false, Test::Goes(Uge), 1, true),
            (false, Test::Goes(Uge), 0, false),
            (false, Test::Leaves(Slt), -3, true),
            (false, Test::Leaves(Slt), least, false),
            (false, Test::Leaves(Ult), 9, true),
            (false, Test::Leaves(Ult), 0, false),
            (false, Test::Swapped(Sle), 7, true),
            (false, Test::Swapped(Ule), 0, false),
        ];
        for (up, test, limit, splits) in cases {
            // First values round the constant: every count from nothing to
            // three trips, and the far side.
            let counts: Vec<(u64, u64)> = (-3..=14)
                .chain([most, least, 100])
                .map(|away: i64| {
                    let first = if up {
                        limit.wrapping_sub(away)
                    } else {
                        limit.wrapping_add(away)
                    };
                    (cut(first as u64, ty), 0)
                })
                .collect();
            for bottom in [false, true] {
                let limbs = Limbs {
                    ty,
                    bottom,
                    pointers: bottom,
                    stepped_first: false,
                    up,
                    test,
                    tests_next: up,
                    limit: Limit::Constant(limit),
                    more: false,
                    narrowed: None,
                    step_in_header: false,
                };
                compared(&limbs, splits, &counts);
                // The same test against the argument: no neighbour is
                // known.
                let against_argument = Limbs {
                    limit: Limit::Argument,
                    ..limbs
                };
                compared(&against_argument, false, &[]);
            }
        }
    }
}

// ── What is left alone ───────────────────────────────────────────────────

/// `func` is not split, and is what it was.
fn left_alone(mut func: IrFunction, why: &str) {
    func.next_value_id = func.sound_next_value_id();
    func.next_label = func
        .blocks
        .iter()
        .map(|block| block.label.0 + 1)
        .max()
        .unwrap_or(0);
    well_formed(&func, &why);
    let before = format!("{func:?}");
    assert_eq!(run(&mut func), 0, "{why}");
    assert_eq!(before, format!("{func:?}"), "{why}");
}

/// The block of `func` with the work in it, and where the work starts.
fn work_of(func: &mut IrFunction, bottom: bool) -> (&mut Vec<Instruction>, usize) {
    let block = &mut func.blocks[if bottom { 1 } else { 2 }].instructions;
    let start = block
        .iter()
        .take_while(|inst| matches!(inst, Instruction::Phi { .. }))
        .count();
    (block, start)
}

#[test]
fn a_loop_without_one_carry_round_it_is_left_alone() {
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        // No carry: the sum is stored, and the phi receives nothing of it.
        let mut func = limbs.build();
        let latch = if limbs.bottom { 1 } else { 2 };
        func.blocks[1].instructions[1] = phi(11, U64, vec![(wide(0), 0), (wide(0), latch)]);
        left_alone(func, "no carry");

        // The carry out is one of the two comparisons alone: not an adder.
        let mut func = limbs.build();
        func.blocks[1].instructions[1] = phi(11, U64, vec![(wide(0), 0), (v(34), latch)]);
        left_alone(func, "half an adder");

        // Two adders a trip: the loop is unrolled already.
        let mut func = limbs.build();
        let (work, _) = work_of(&mut func, limbs.bottom);
        let at = work
            .iter()
            .position(|inst| matches!(inst, Instruction::Store { .. }))
            .expect("the store")
            + 1;
        work.splice(
            at..at,
            [
                bin(132, IrBinOp::Add, v(31), v(30), U64),
                cmp(133, IrCmpOp::Ult, v(132), v(31), U64),
                cast(134, 133, IrType::I32, U64),
                bin(135, IrBinOp::Add, v(132), v(38), U64),
                cmp(136, IrCmpOp::Ult, v(135), v(132), U64),
                cast(137, 136, IrType::I32, U64),
                bin(138, IrBinOp::Or, v(134), v(137), U64),
                store(v(135), 2),
            ],
        );
        func.blocks[1].instructions[1] = phi(11, U64, vec![(wide(0), 0), (v(138), latch)]);
        left_alone(func, "two adders a trip");
    }
}

#[test]
fn work_that_is_more_than_loads_stores_and_arithmetic_is_left_alone() {
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        let mut func = limbs.build();
        let (work, _) = work_of(&mut func, limbs.bottom);
        for inst in work.iter_mut() {
            if let Instruction::Load { volatile, .. } = inst {
                *volatile = true;
                break;
            }
        }
        left_alone(func, "a volatile load");

        let mut func = limbs.build();
        let (work, _) = work_of(&mut func, limbs.bottom);
        for inst in work.iter_mut() {
            if let Instruction::Store { volatile, .. } = inst {
                *volatile = true;
            }
        }
        left_alone(func, "a volatile store");

        let mut func = limbs.build();
        let (work, start) = work_of(&mut func, limbs.bottom);
        work.insert(start, Instruction::StackSave { dest: Value(150) });
        left_alone(func, "something else");

        // More work than is copied.
        let mut func = limbs.build();
        let (work, start) = work_of(&mut func, limbs.bottom);
        let padding = (0..MAX_WORK as u32)
            .map(|at| bin(200 + at, IrBinOp::Xor, v(11), wide(i64::from(at)), U64));
        work.splice(start..start, padding);
        left_alone(func, "a long body");
    }
}

/// `func` with one more phi in its header, `phi`, which receives `from_trip`
/// round the loop, and `more` behind the store of the limb.
fn with_work(limbs: &Limbs, phi_id: u32, from_trip: Operand, more: Vec<Instruction>) -> IrFunction {
    let mut func = limbs.build();
    let latch = if limbs.bottom { 1 } else { 2 };
    func.blocks[1]
        .instructions
        .insert(2, phi(phi_id, U64, vec![(wide(0), 0), (from_trip, latch)]));
    let (work, _) = work_of(&mut func, limbs.bottom);
    let at = work
        .iter()
        .position(|inst| matches!(inst, Instruction::Store { .. }))
        .expect("the store")
        + 1;
    work.splice(at..at, more);
    func.next_value_id = func.sound_next_value_id();
    func
}

/// A trip that does more than its limb -- sums the limbs, compares them,
/// shifts them -- writes the flags between two adders, and the carry has
/// to leave the flag for it: four copies of that are four times the loop
/// and no chain. Such a loop is left as it is.
#[test]
fn a_trip_that_is_more_than_a_limb_is_left_alone() {
    use IrBinOp::{Add, And, Mul, Shl, Xor};
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        // `h += t`, `h ^= t`, `h &= t`: a sum over the limbs.
        for op in [Add, Xor, And] {
            let func = with_work(&limbs, 117, v(146), vec![bin(146, op, v(117), v(35), U64)]);
            left_alone(func, "a sum over the limbs");
        }
        // A word of the limb's own arithmetic kept: `h = a[i] + b[i]`.
        let func = with_work(&limbs, 117, v(32), vec![]);
        left_alone(func, "half the sum is kept");
        // A compare of the data that is not the loop's test: `top = t < x`.
        let func = with_work(
            &limbs,
            117,
            v(147),
            vec![
                cmp(146, IrCmpOp::Ult, v(35), v(30), U64),
                cast(147, 146, IrType::I32, U64),
            ],
        );
        left_alone(func, "a compare of the limbs");
        // The limb shifted before it is kept: a shift is an address only
        // where an address reads it.
        let func = with_work(
            &limbs,
            117,
            v(146),
            vec![bin(146, Shl, v(35), wide(3), U64)],
        );
        left_alone(func, "a shift of a limb");
        // A second counter by something that is not a constant.
        let func = with_work(&limbs, 117, v(146), vec![bin(146, Add, v(117), v(30), U64)]);
        left_alone(func, "a step by a limb");

        // The same work with nobody to read it is no work: the loop is
        // split, and runs as before.
        let mut func = with_work(&limbs, 117, wide(3), vec![bin(146, Xor, v(35), v(30), U64)]);
        assert_eq!(run(&mut func), 1, "work nobody reads");
        well_formed(&func, &"work nobody reads");
    }

    // An index scaled by something an address does not scale by.
    let mut func = Limbs::mini().build();
    let (work, _) = work_of(&mut func, false);
    assert!(matches!(work[0], Instruction::BinOp { op: Shl, .. }));
    work[0] = bin(19, Mul, v(10), wide(24), I64);
    left_alone(func, "an index times 24");
    // An index that is masked first: an `and` is no part of an address.
    let mut func = Limbs::mini().build();
    let (work, _) = work_of(&mut func, false);
    work.insert(0, bin(118, And, v(10), wide(7), I64));
    work[1] = bin(19, Shl, v(118), wide(3), I64);
    left_alone(func, "an index that is masked");
    // The scaled index is data as well: stored, it has to be computed.
    let mut func = Limbs::mini().build();
    let (work, _) = work_of(&mut func, false);
    work.push(store(v(19), 22));
    left_alone(func, "a scaled index that is stored");
    // An index that is the difference of two things the loop does not
    // know apart: `a[n - i]`.
    let mut func = Limbs::mini().build();
    let (work, _) = work_of(&mut func, false);
    work.insert(0, bin(118, IrBinOp::Sub, v(5), v(10), I64));
    work[1] = bin(19, Shl, v(118), wide(3), I64);
    left_alone(func, "an index counted from the other end");
}

/// What an access folds into its address, and a move, are part of a limb:
/// an index moved by a constant, scaled by the size of a word in either
/// spelling, a copy of the sum.
#[test]
fn address_arithmetic_and_moves_are_part_of_a_limb() {
    use IrBinOp::{Add, Mul, Shl, Sub};
    let counts = counters(I64, true);
    let mini = Limbs::mini();

    // `a[i + 2]`, `a[i - 1]`.
    for (op, by) in [(Add, 2), (Sub, 1)] {
        let mut func = mini.build();
        let (work, _) = work_of(&mut func, false);
        work.insert(0, bin(118, op, v(10), wide(by), I64));
        work[1] = bin(19, Shl, v(118), wide(3), I64);
        func.next_value_id = func.sound_next_value_id();
        compared_built(func, &"an index moved by a constant", true, &counts);
    }
    // `i * 8` for `i << 3`.
    let mut func = mini.build();
    let (work, _) = work_of(&mut func, false);
    work[0] = bin(19, Mul, v(10), wide(8), I64);
    compared_built(func, &"an index times eight", true, &counts);
    // The scaled index moved: `(i << 3) + 16`.
    let mut func = mini.build();
    let (work, _) = work_of(&mut func, false);
    work[0] = bin(118, Shl, v(10), wide(3), I64);
    work.insert(1, bin(19, Add, v(118), wide(16), I64));
    func.next_value_id = func.sound_next_value_id();
    compared_built(func, &"a scaled index moved", true, &counts);
    // A copy of the sum is what is stored, and a copy of a limb is what
    // is added.
    for limbs in [Limbs::gmp(), mini] {
        let mut func = limbs.build();
        let (work, _) = work_of(&mut func, limbs.bottom);
        let at = work
            .iter()
            .position(|inst| matches!(inst, Instruction::Store { .. }))
            .expect("the store");
        let Instruction::Store { val, .. } = &mut work[at] else {
            unreachable!()
        };
        *val = v(118);
        work.insert(
            at,
            Instruction::Copy {
                dest: Value(118),
                src: v(35),
            },
        );
        func.next_value_id = func.sound_next_value_id();
        let counts: Vec<(u64, u64)> = if limbs.bottom {
            (1..=13).map(|n| (n, 0)).collect()
        } else {
            counts.clone()
        };
        compared_built(func, &"a copy of the sum", true, &counts);
    }
}

#[test]
fn a_counter_that_is_not_stepped_by_one_towards_a_fixed_limit_is_left_alone() {
    use IrCmpOp::{Sgt, Slt, Ugt, Ult};
    // By two.
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        let mut func = limbs.build();
        let (work, _) = work_of(&mut func, limbs.bottom);
        for inst in work.iter_mut() {
            if let Instruction::BinOp { dest, rhs, .. } = inst
                && *dest == Value(40)
            {
                *rhs = typed(2, limbs.ty);
            }
        }
        left_alone(func, "a step of two");
    }
    // Away from the limit: `i > n` while `i` goes up, `i < n` down.
    for (up, test) in [
        (true, Test::Goes(Sgt)),
        (true, Test::Goes(Ugt)),
        (true, Test::Swapped(Slt)),
        (true, Test::Leaves(IrCmpOp::Sle)),
        (false, Test::Goes(Slt)),
        (false, Test::Goes(Ult)),
        (false, Test::Swapped(Ugt)),
        (false, Test::Leaves(IrCmpOp::Uge)),
        (true, Test::Goes(IrCmpOp::Eq)),
        (false, Test::Leaves(IrCmpOp::Ne)),
    ] {
        for bottom in [false, true] {
            let limbs = Limbs {
                up,
                test,
                bottom,
                tests_next: bottom,
                limit: Limit::Argument,
                ..Limbs::mini()
            };
            left_alone(limbs.build(), "a test away from the step");
        }
    }
    // A counter of 16 bits.
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        let narrow = Limbs {
            ty: IrType::U16,
            ..limbs
        };
        left_alone(narrow.build(), "a 16-bit counter");
    }
    // A limit the loop computes.
    let mut func = Limbs::mini().build();
    func.blocks[1].instructions[2] = cmp(41, Slt, v(10), v(11), I64);
    left_alone(func, "a limit that is a phi");
    let mut func = Limbs {
        limit: Limit::Argument,
        ..Limbs::gmp()
    }
    .build();
    let (work, _) = work_of(&mut func, true);
    let test = work.len() - 1;
    work[test] = cmp(41, IrCmpOp::Ne, v(10), v(30), I64);
    left_alone(func, "a limit that is loaded in the loop");
    // The test is in another width than the counter.
    let mut func = Limbs::mini().build();
    func.blocks[1].instructions[2] = cmp(41, Slt, v(10), v(5), IrType::I32);
    left_alone(func, "a 32-bit test of a 64-bit counter");
    // The counter's own step is in another width.
    let mut func = Limbs::mini().build();
    let (work, _) = work_of(&mut func, false);
    let step = work.len() - 1;
    work[step] = bin(40, IrBinOp::Add, v(10), wide(1), IrType::I32);
    left_alone(func, "a 32-bit step of a 64-bit counter");
}

#[test]
fn a_loop_of_another_shape_is_left_alone() {
    // The header of a loop tested at the top holds more than its test.
    let mut func = Limbs::mini().build();
    func.blocks[1]
        .instructions
        .insert(2, bin(150, IrBinOp::Xor, v(11), wide(1), U64));
    left_alone(func, "work in the header");

    // The work reads the header's test.
    let mut func = Limbs::mini().build();
    let (work, start) = work_of(&mut func, false);
    work.insert(start, cast(150, 41, IrType::I32, U64));
    left_alone(func, "the test read by the work");

    // A second way in.
    for limbs in [Limbs::gmp(), Limbs::mini()] {
        let mut func = limbs.build();
        let latch = if limbs.bottom { 1 } else { 2 };
        func.blocks[0].terminator = Terminator::CondBranch {
            cond: v(4),
            true_label: BlockId(1),
            false_label: BlockId(9),
        };
        func.blocks
            .push(block(BlockId(9), vec![], Terminator::Branch(BlockId(1))));
        for inst in &mut func.blocks[1].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                let from_entry = incoming
                    .iter()
                    .find(|(_, from)| *from == BlockId(0))
                    .expect("the way in")
                    .0
                    .clone();
                incoming.push((from_entry, BlockId(9)));
                assert!(incoming.iter().any(|(_, from)| *from == BlockId(latch)));
            }
        }
        left_alone(func, "two ways in");
    }

    // Two blocks of work.
    let mut func = Limbs::mini().build();
    let tail = func.blocks[2].instructions.split_off(6);
    func.blocks[2].terminator = Terminator::Branch(BlockId(9));
    func.blocks
        .push(block(BlockId(9), tail, Terminator::Branch(BlockId(1))));
    for inst in &mut func.blocks[1].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            for (_, from) in incoming.iter_mut() {
                if *from == BlockId(2) {
                    *from = BlockId(9);
                }
            }
        }
    }
    left_alone(func, "two blocks of work");

    // A second way out.
    let mut func = Limbs::mini().build();
    func.blocks[2].terminator = Terminator::CondBranch {
        cond: v(38),
        true_label: BlockId(1),
        false_label: BlockId(9),
    };
    func.blocks
        .push(block(BlockId(9), vec![], Terminator::Return(Some(wide(0)))));
    left_alone(func, "two ways out");

    // Both ways of the test stay in the loop.
    let mut func = Limbs::gmp().build();
    func.blocks[1].terminator = Terminator::CondBranch {
        cond: v(41),
        true_label: BlockId(1),
        false_label: BlockId(1),
    };
    left_alone(func, "no way out");
}

// ── The copies ───────────────────────────────────────────────────────────

/// A value the loop computes and something behind it reads comes from the
/// last copy when the loop as it was does not run: one phi a value, in a
/// block in front of the exit.
#[test]
fn what_is_read_behind_a_loop_tested_at_the_bottom_is_joined() {
    let limbs = Limbs {
        stepped_first: true,
        more: true,
        ..Limbs::gmp()
    };
    let mut func = limbs.build();
    assert_eq!(run(&mut func), 1);
    well_formed(&func, &limbs);
    let join = func.blocks.last().expect("the join");
    // The carry, the counter, the pointer and the four more phis; the
    // carry out, the stepped counter, the sum, the stepped second counter
    // and the stepped pointer.
    assert_eq!(join.instructions.len(), 12);
    assert!(
        join.instructions
            .iter()
            .all(|inst| matches!(inst, Instruction::Phi { incoming, .. } if incoming.len() == 2))
    );
    assert!(matches!(join.terminator, Terminator::Branch(BlockId(3))));
    // Behind the loop nothing of the loop is read by its own name.
    let mut read = Vec::new();
    for inst in &func.blocks[2].instructions {
        inst.for_each_used_value(|value| read.push(value));
    }
    assert!(
        read.iter().all(|value| *value >= 60),
        "{:?}",
        func.blocks[2].instructions
    );
}

/// A counter that only counts is not carried through the main loop; one
/// that is read is.
#[test]
fn a_counter_that_only_counts_is_what_is_left_of_the_count() {
    // GMP's loop reads `n` for nothing but `--n != 0`. Without a reader
    // behind the loop, the loop as it was starts from `n & 3`.
    let mut func = Limbs::gmp().build();
    // Behind the loop only the carry is read.
    func.blocks[2].instructions.clear();
    func.blocks[2].terminator = Terminator::Return(Some(v(38)));
    assert_eq!(run(&mut func), 1);
    well_formed(&func, &"gmp");
    let from_main = |func: &IrFunction, phi: u32| -> Value {
        func.blocks[1]
            .instructions
            .iter()
            .find_map(|inst| match inst {
                Instruction::Phi { dest, incoming, .. } if dest.0 == phi => {
                    assert_eq!(incoming.len(), 3);
                    match incoming[2].0 {
                        Operand::Value(value) => Some(value),
                        Operand::Const(_) => None,
                    }
                }
                _ => None,
            })
            .expect("a third way in")
    };
    let rest = from_main(&func, 10);
    let defined = func
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|inst| inst.dest() == Some(rest))
        .expect("the counter behind the main loop");
    assert!(
        matches!(
            defined,
            Instruction::BinOp {
                op: IrBinOp::And,
                rhs: Operand::Const(three),
                ..
            } if three.to_i64() == Some(3)
        ),
        "{defined:?}"
    );
    // The count of trips is the guard's, which branches into the main loop
    // through a block that computes nothing.
    let guard = &func.blocks[3];
    assert!(
        guard.instructions.iter().any(|inst| matches!(
            inst,
            Instruction::BinOp {
                op: IrBinOp::LShr,
                rhs: Operand::Const(two),
                ..
            } if two.to_i64() == Some(2)
        )),
        "{:?}",
        guard.instructions
    );
    assert!(func.blocks[4].instructions.is_empty());

    // mini-gmp's loop indexes with it: the copies step it.
    let mut func = Limbs::mini().build();
    assert_eq!(run(&mut func), 1);
    let index = from_main(&func, 10);
    let main = main_loop(&func);
    let main = func
        .blocks
        .iter()
        .find(|block| block.label.0 == main)
        .expect("the main loop");
    assert!(
        main.instructions.iter().any(|inst| matches!(
            inst,
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                rhs: Operand::Const(four),
                ..
            } if *dest == index && four.to_i64() == Some(4)
        )),
        "{:?}",
        main.instructions
    );
    // Its trips are counted on the way in, not in the guard.
    assert!(
        func.blocks[4].instructions.iter().all(|inst| !matches!(
            inst,
            Instruction::BinOp {
                op: IrBinOp::LShr,
                ..
            }
        )),
        "{:?}",
        func.blocks[4].instructions
    );
    assert!(matches!(
        func.blocks[5].instructions[..],
        [Instruction::BinOp {
            op: IrBinOp::LShr,
            ..
        }]
    ));
}

/// `_addcarry_u64(c, a[i], b[i], &r[i])` computes every address ahead of
/// the loads. In a copy each one stands directly in front of the load or
/// the store that goes through it, so that it is not one more value alive
/// at the adder; the loop as it was keeps the order it had.
#[test]
fn an_address_stands_in_front_of_the_access_it_is_for() {
    let limbs = Limbs::mini();
    let before = limbs.build();
    let mut func = before.clone();
    assert_eq!(run(&mut func), 1);
    let main = main_loop(&func);
    let main = func
        .blocks
        .iter()
        .find(|block| block.label.0 == main)
        .expect("the main loop");
    let mut addresses = 0;
    for pair in main.instructions.windows(2) {
        let Instruction::GetElementPtr {
            dest,
            offset: Operand::Value(_),
            ..
        } = &pair[0]
        else {
            continue;
        };
        addresses += 1;
        assert!(
            matches!(
                &pair[1],
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } if ptr == dest
            ),
            "{:?}",
            pair
        );
    }
    assert_eq!(addresses, 3 * COPIES as usize);
    // The loop as it was: all three addresses, then the loads.
    assert_eq!(
        format!("{:?}", before.blocks[2].instructions),
        format!("{:?}", func.blocks[2].instructions)
    );
    assert!(matches!(
        func.blocks[2].instructions[..4],
        [
            Instruction::BinOp { .. },
            Instruction::GetElementPtr { .. },
            Instruction::GetElementPtr { .. },
            Instruction::GetElementPtr { .. },
        ]
    ));
}
