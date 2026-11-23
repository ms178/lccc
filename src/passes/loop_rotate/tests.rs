//! Tests of loop rotation: the transformation over every loop shape it
//! takes, the refusals, and both policies.
//!
//! The oracle is behavioural. Each fixture is run by a small interpreter
//! before and after the rotation over a grid of inputs that covers zero, one
//! and many trips, and the two results have to agree; the rotated function
//! also has to pass the IR verifier, which is what catches a reader that
//! kept the wrong definition of a header value (a use its definition does
//! not dominate, a phi without an incoming for one of its edges).

use super::*;
use crate::common::types::{AddressSpace, IrType as Ty};
use crate::ir::reexports::{IrBinOp, IrCmpOp};

fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        instructions: insts,
        terminator: term,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn k(n: i64) -> Operand {
    Operand::Const(IrConst::I64(n))
}

fn param(dest: u32, index: usize) -> Instruction {
    Instruction::ParamRef {
        dest: Value(dest),
        param_idx: index,
        ty: Ty::I64,
    }
}

fn phi(dest: u32, arms: &[(Operand, u32)]) -> Instruction {
    Instruction::Phi {
        dest: Value(dest),
        ty: Ty::I64,
        incoming: arms.iter().map(|&(op, from)| (op, BlockId(from))).collect(),
    }
}

fn bin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty: Ty::I64,
    }
}

fn add(dest: u32, lhs: Operand, rhs: Operand) -> Instruction {
    bin(dest, IrBinOp::Add, lhs, rhs)
}

fn mul(dest: u32, lhs: Operand, rhs: Operand) -> Instruction {
    bin(dest, IrBinOp::Mul, lhs, rhs)
}

fn cmp(dest: u32, op: IrCmpOp, lhs: Operand, rhs: Operand) -> Instruction {
    Instruction::Cmp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty: Ty::I64,
    }
}

fn load(dest: u32, ptr: u32) -> Instruction {
    Instruction::Load {
        dest: Value(dest),
        ptr: Value(ptr),
        ty: Ty::I64,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

fn store(val: Operand, ptr: u32) -> Instruction {
    Instruction::Store {
        val,
        ptr: Value(ptr),
        ty: Ty::I64,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

fn br(to: u32) -> Terminator {
    Terminator::Branch(BlockId(to))
}

fn cbr(cond: Operand, on_true: u32, on_false: u32) -> Terminator {
    Terminator::CondBranch {
        cond,
        true_label: BlockId(on_true),
        false_label: BlockId(on_false),
    }
}

fn ret(op: Operand) -> Terminator {
    Terminator::Return(Some(op))
}

fn func(blocks: Vec<BasicBlock>) -> IrFunction {
    let mut f = IrFunction::new("rot".into(), Ty::I64, vec![], false);
    f.blocks = blocks;
    // The two counters every pass may trust (the verifier holds the label
    // counter against the labels in use).
    f.next_label = f.blocks.iter().map(|b| b.label.0 + 1).max().unwrap_or(1);
    f.next_value_id = f.sound_next_value_id();
    f
}

/// Run `f` on `params`. `None` when it does not return within its fuel, so a
/// loop that never ends compares equal to itself and cannot hang the suite.
/// Phis of a block read simultaneously; a pointer is a cell number; reading
/// a value no executed instruction defined is a failure of the test.
fn run(f: &IrFunction, params: &[i64]) -> Option<i64> {
    let mut values: FxHashMap<u32, i64> = FxHashMap::default();
    let mut memory: FxHashMap<i64, i64> = FxHashMap::default();
    let read = |op: Operand, values: &FxHashMap<u32, i64>| match op {
        Operand::Const(c) => c.to_i64().expect("an integer constant"),
        Operand::Value(x) => *values
            .get(&x.0)
            .unwrap_or_else(|| panic!("v{} is read before anything defined it", x.0)),
    };
    let mut at = f.blocks[0].label;
    let mut from = at;
    for _ in 0..4096 {
        let block = f
            .blocks
            .iter()
            .find(|b| b.label == at)
            .expect("a branch to a block of the function");
        let merged: Vec<(u32, i64)> = block
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::Phi { dest, incoming, .. } => {
                    let arm = incoming
                        .iter()
                        .find(|(_, label)| *label == from)
                        .unwrap_or_else(|| {
                            panic!("phi v{} has no incoming for .LBB{}", dest.0, from.0)
                        });
                    Some((dest.0, read(arm.0, &values)))
                }
                _ => None,
            })
            .collect();
        values.extend(merged);
        for inst in &block.instructions {
            match inst {
                Instruction::Phi { .. } => {}
                Instruction::ParamRef {
                    dest, param_idx, ..
                } => {
                    values.insert(dest.0, params[*param_idx]);
                }
                Instruction::Copy { dest, src } => {
                    let x = read(*src, &values);
                    values.insert(dest.0, x);
                }
                Instruction::BinOp {
                    dest, op, lhs, rhs, ..
                } => {
                    let (l, r) = (read(*lhs, &values), read(*rhs, &values));
                    let x = match op {
                        IrBinOp::Add => l.wrapping_add(r),
                        IrBinOp::Sub => l.wrapping_sub(r),
                        IrBinOp::Mul => l.wrapping_mul(r),
                        IrBinOp::And => l & r,
                        IrBinOp::Xor => l ^ r,
                        IrBinOp::SDiv => {
                            assert_ne!(
                                r, 0,
                                "a fixture divided by zero: the division ran where it must not"
                            );
                            l.wrapping_div(r)
                        }
                        other => panic!("the interpreter has no {other:?}"),
                    };
                    values.insert(dest.0, x);
                }
                Instruction::Cmp {
                    dest, op, lhs, rhs, ..
                } => {
                    let (l, r) = (read(*lhs, &values), read(*rhs, &values));
                    let truth = match op {
                        IrCmpOp::Slt => l < r,
                        IrCmpOp::Sle => l <= r,
                        IrCmpOp::Sgt => l > r,
                        IrCmpOp::Sge => l >= r,
                        IrCmpOp::Eq => l == r,
                        IrCmpOp::Ne => l != r,
                        other => panic!("the interpreter has no {other:?}"),
                    };
                    values.insert(dest.0, i64::from(truth));
                }
                Instruction::Load { dest, ptr, .. } => {
                    let cell = read(Operand::Value(*ptr), &values);
                    values.insert(dest.0, memory.get(&cell).copied().unwrap_or(0));
                }
                Instruction::Store { val, ptr, .. } => {
                    let cell = read(Operand::Value(*ptr), &values);
                    let x = read(*val, &values);
                    memory.insert(cell, x);
                }
                other => panic!("the interpreter has no {other:?}"),
            }
        }
        from = at;
        at = match &block.terminator {
            Terminator::Branch(next) => *next,
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                if read(*cond, &values) != 0 {
                    *true_label
                } else {
                    *false_label
                }
            }
            Terminator::Return(Some(op)) => return Some(read(*op, &values)),
            other => panic!("the interpreter has no {other:?}"),
        };
    }
    None
}

/// Inputs for a fixture with parameters `(seed, bound, extra)`: no trip, one
/// trip, several, and bounds below the seed.
fn grid() -> Vec<[i64; 3]> {
    let mut inputs = Vec::new();
    for seed in [-2, 0, 1, 5] {
        for bound in [-3, 0, 1, 2, 6, 11] {
            for extra in [0, 1, 3] {
                inputs.push([seed, bound, extra]);
            }
        }
    }
    inputs
}

fn violations(f: &IrFunction) -> Vec<String> {
    let mut found = Vec::new();
    crate::passes::verify::verify_function(f, "loop_rotate", &mut found);
    found.iter().map(ToString::to_string).collect()
}

/// Rotate every loop of the fixture and hold the result against the
/// fixture: `rotations` loops rotated, valid IR, the same result on every
/// input of the grid. Returns the rotated function.
fn rotated(what: &str, build: &dyn Fn() -> IrFunction, rotations: usize) -> IrFunction {
    let before = build();
    assert_eq!(
        violations(&before),
        Vec::<String>::new(),
        "{what}: the fixture"
    );
    let mut after = build();
    assert_eq!(
        rotate_every_loop(&mut after),
        rotations,
        "{what}: loops rotated"
    );
    assert_eq!(violations(&after), Vec::<String>::new(), "{what}: rotated");
    for input in grid() {
        assert_eq!(
            run(&after, &input),
            run(&before, &input),
            "{what}: on {input:?}"
        );
    }
    // A second run finds nothing left to do: the loops are bottom-tested.
    let again = format!("{after:?}");
    assert_eq!(rotate_every_loop(&mut after), 0, "{what}: rotated twice");
    assert_eq!(
        format!("{after:?}"),
        again,
        "{what}: changed by a run that rotated nothing"
    );
    after
}

fn count_phis(f: &IrFunction) -> usize {
    f.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter(|i| matches!(i, Instruction::Phi { .. }))
        .count()
}

/// The three parameters every fixture starts with: v1 = seed, v2 = bound,
/// v3 = extra.
fn params() -> Vec<Instruction> {
    vec![param(1, 0), param(2, 1), param(3, 2)]
}

// ── The policy switch ───────────────────────────────────────────────────────

#[test]
fn the_policy_is_what_the_two_switches_say() {
    use RotatePolicy::{Always, Off, Profit};
    let cases: &[(bool, Option<&str>, RotatePolicy)] = &[
        (false, None, Profit),
        (false, Some(""), Profit),
        (false, Some("  "), Profit),
        (false, Some("profit"), Profit),
        (false, Some("sideways"), Profit),
        (false, Some("0"), Off),
        (false, Some("off"), Off),
        (false, Some("No"), Off),
        (false, Some("false"), Off),
        (false, Some("1"), Always),
        (false, Some(" 1 "), Always),
        (false, Some("TRUE"), Always),
        (false, Some("yes"), Always),
        (false, Some("on"), Always),
        (false, Some("always"), Always),
        // The kill switch wins over whatever the other one says.
        (true, None, Off),
        (true, Some("1"), Off),
        (true, Some("always"), Off),
    ];
    for &(kill, choice, expected) in cases {
        assert_eq!(
            RotatePolicy::resolve(kill, choice),
            expected,
            "{kill} {choice:?}"
        );
    }
}

// ── One loop, one body block ────────────────────────────────────────────────

/// ```text
///   0: params                          -> 1
///   1: i = phi [seed, 0] [i', 2]
///      t = i + 3
///      c = t OP bound                  -> 2 | 3   (or 3 | 2)
///   2: i' = i + 1                      -> 1
///   3: ret RESULT
/// ```
/// `result` picks what is read behind the loop: the phi (10), the header's
/// own instruction (11), or a constant.
fn counted(op: IrCmpOp, bound_left: bool, exit_on_true: bool, result: Operand) -> IrFunction {
    let (lhs, rhs) = if bound_left {
        (v(2), v(11))
    } else {
        (v(11), v(2))
    };
    let term = if exit_on_true {
        cbr(v(12), 3, 2)
    } else {
        cbr(v(12), 2, 3)
    };
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(13), 2)]),
                add(11, v(10), k(3)),
                cmp(12, op, lhs, rhs),
            ],
            term,
        ),
        blk(2, vec![add(13, v(10), k(1))], br(1)),
        blk(3, vec![], ret(result)),
    ])
}

#[test]
fn a_counted_loop_rotates_in_both_polarities_and_operand_orders() {
    for op in [
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ne,
    ] {
        for bound_left in [false, true] {
            for exit_on_true in [false, true] {
                for result in [v(10), v(11), k(7)] {
                    let what = format!(
                        "{op:?} bound_left={bound_left} exit_on_true={exit_on_true} {result:?}"
                    );
                    let after =
                        rotated(&what, &|| counted(op, bound_left, exit_on_true, result), 1);
                    // The latch carries the copy of the test and branches
                    // the way the header does.
                    let latch = after.blocks.iter().find(|b| b.label == BlockId(2)).unwrap();
                    let Terminator::CondBranch {
                        true_label,
                        false_label,
                        ..
                    } = latch.terminator
                    else {
                        panic!("{what}: the latch does not end in the test");
                    };
                    let (stay, leave) = if exit_on_true {
                        (false_label, true_label)
                    } else {
                        (true_label, false_label)
                    };
                    assert_eq!(stay, BlockId(2), "{what}: the back edge");
                    assert_eq!(leave, BlockId(3), "{what}: the exit edge");
                }
            }
        }
    }
}

#[test]
fn the_guard_keeps_one_test_and_the_latch_gets_one() {
    let after = rotated("counted", &|| counted(IrCmpOp::Slt, false, false, v(10)), 1);
    let compares = |label: u32| {
        after
            .blocks
            .iter()
            .find(|b| b.label == BlockId(label))
            .unwrap()
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Cmp { .. }))
            .count()
    };
    assert_eq!((compares(1), compares(2)), (1, 1));
    // The preheader sits between the guard and the loop and holds nothing.
    let guard = after
        .blocks
        .iter()
        .position(|b| b.label == BlockId(1))
        .unwrap();
    let preheader = &after.blocks[guard + 1];
    assert!(preheader.instructions.is_empty());
    assert!(matches!(preheader.terminator, Terminator::Branch(to) if to == BlockId(2)));
    // One merge for `i` in the loop, one behind it for the reader there.
    // The guard's own phi has one way in left and is a copy.
    assert_eq!(count_phis(&after), 2, "{after:#?}");
    let guard = &after.blocks[guard];
    assert!(
        matches!(
            guard.instructions[0],
            Instruction::Copy {
                dest: Value(10),
                src: Operand::Value(Value(1))
            }
        ),
        "{guard:?}"
    );
}

#[test]
fn a_value_nobody_reads_behind_the_loop_gets_no_merge_there() {
    let after = rotated("counted", &|| counted(IrCmpOp::Slt, false, false, k(7)), 1);
    let exit = after.blocks.iter().find(|b| b.label == BlockId(3)).unwrap();
    assert!(exit.instructions.is_empty(), "{exit:?}");
}

/// The body reads what the header computed (`t`), the latch hands the phi a
/// value the header computed (`d`: the `while (--i)` shape), and both are
/// read behind the loop.
///
/// ```text
///   1: i = phi [seed, 0] [d, 2]   acc = phi [0, 0] [acc', 2]
///      d = i + -1
///      t = d * 3
///      c = d > bound               -> 2 | 3
///   2: acc' = acc + t              -> 1
///   3: r = acc + d ; r2 = r + t ; ret r2
/// ```
fn header_values_everywhere() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(11), 2)]),
                phi(20, &[(k(0), 0), (v(21), 2)]),
                add(11, v(10), k(-1)),
                mul(12, v(11), k(3)),
                cmp(13, IrCmpOp::Sgt, v(11), v(2)),
            ],
            cbr(v(13), 2, 3),
        ),
        blk(2, vec![add(21, v(20), v(12))], br(1)),
        blk(
            3,
            vec![add(30, v(20), v(11)), add(31, v(30), v(12))],
            ret(v(31)),
        ),
    ])
}

#[test]
fn every_reader_of_a_header_value_sees_its_own_trip() {
    rotated("header values", &header_values_everywhere, 1);
}

/// Two phis that swap, and a third that takes one of them: what each holds
/// behind the loop is what the last test saw.
///
/// ```text
///   1: a = phi [seed, 0] [b, 2]   b = phi [extra, 0] [t, 2]   i = phi [0, 0] [i', 2]
///      c = i < bound               -> 2 | 3
///   2: t = a + b ; i' = i + 1      -> 1
///   3: r = a * 7 ; r2 = r + b ; ret r2
/// ```
fn swapped_phis() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(11), 2)]),
                phi(11, &[(v(3), 0), (v(20), 2)]),
                phi(12, &[(k(0), 0), (v(21), 2)]),
                cmp(13, IrCmpOp::Slt, v(12), v(2)),
            ],
            cbr(v(13), 2, 3),
        ),
        blk(2, vec![add(20, v(10), v(11)), add(21, v(12), k(1))], br(1)),
        blk(
            3,
            vec![mul(30, v(10), k(7)), add(31, v(30), v(11))],
            ret(v(31)),
        ),
    ])
}

#[test]
fn phis_that_feed_each_other_keep_their_order() {
    rotated("swapped phis", &swapped_phis, 1);
}

/// A phi the back edge resets to a constant, read by the test. The constant
/// is what the bottom test sees in the phi's place.
fn reset_on_the_back_edge() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (k(4), 2)]),
                phi(20, &[(k(0), 0), (v(21), 2)]),
                add(11, v(10), v(20)),
                cmp(12, IrCmpOp::Slt, v(11), v(2)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(2, vec![add(21, v(20), k(1))], br(1)),
        blk(3, vec![add(30, v(10), v(20))], ret(v(30))),
    ])
}

#[test]
fn a_constant_over_the_back_edge_is_what_the_bottom_test_reads() {
    rotated("reset", &reset_on_the_back_edge, 1);
}

/// The header loads its bound and the body stores a new one: the copy of
/// the load in the latch has to run after the store, every trip.
///
/// ```text
///   0: p = extra (a cell) ; store bound -> p                     -> 1
///   1: i = phi [seed, 0] [i', 2]
///      lim = load p
///      c = i < lim                                                -> 2 | 3
///   2: i' = i + 1 ; lim' = lim + -1 ; store lim' -> p             -> 1
///   3: r = i * 10 ; r2 = r + lim ; ret r2
/// ```
fn header_load() -> IrFunction {
    func(vec![
        blk(
            0,
            {
                let mut insts = params();
                insts.push(store(v(2), 3));
                insts
            },
            br(1),
        ),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(20), 2)]),
                load(11, 3),
                cmp(12, IrCmpOp::Slt, v(10), v(11)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(
            2,
            vec![add(20, v(10), k(1)), add(21, v(11), k(-1)), store(v(21), 3)],
            br(1),
        ),
        blk(
            3,
            vec![mul(30, v(10), k(10)), add(31, v(30), v(11))],
            ret(v(31)),
        ),
    ])
}

#[test]
fn a_load_in_the_header_is_read_again_every_trip() {
    let after = rotated("header load", &header_load, 1);
    let loads = after
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter(|i| matches!(i, Instruction::Load { .. }))
        .count();
    assert_eq!(loads, 2);
}

// ── Bodies of several blocks ────────────────────────────────────────────────

/// A body that branches and joins, reading the header's values on both
/// arms.
///
/// ```text
///   1: i = phi [seed, 0] [i', 5]   acc = phi [0, 0] [acc', 5]
///      t = i + extra
///      c = i < bound                    -> 2 | 6
///   2: odd = i & 1 ; z = odd == 0       -> 3 | 4
///   3: x = acc + t                      -> 5
///   4: y = acc * 2                      -> 5
///   5: acc' = phi [x, 3] [y, 4] ; i' = i + 1   -> 1
///   6: r = acc + t ; ret r
/// ```
fn diamond_body() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(51), 5)]),
                phi(20, &[(k(0), 0), (v(50), 5)]),
                add(11, v(10), v(3)),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 6),
        ),
        blk(
            2,
            vec![
                bin(21, IrBinOp::And, v(10), k(1)),
                cmp(22, IrCmpOp::Eq, v(21), k(0)),
            ],
            cbr(v(22), 3, 4),
        ),
        blk(3, vec![add(30, v(20), v(11))], br(5)),
        blk(4, vec![mul(40, v(20), k(2))], br(5)),
        blk(
            5,
            vec![phi(50, &[(v(30), 3), (v(40), 4)]), add(51, v(10), k(1))],
            br(1),
        ),
        blk(6, vec![add(60, v(20), v(11))], ret(v(60))),
    ])
}

#[test]
fn a_body_of_several_blocks_rotates() {
    let after = rotated("diamond", &diamond_body, 1);
    // The join inside the body needs no merge of a header value: both arms
    // are reached from the loop's header alone.
    let join = after.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
    let phis = join
        .instructions
        .iter()
        .filter(|i| matches!(i, Instruction::Phi { .. }))
        .count();
    assert_eq!(phis, 1, "{join:?}");
}

/// Two back edges (`continue`): the bottom test goes into a new block both
/// reach, with a phi for the header phi they disagree on.
///
/// ```text
///   1: i = phi [seed, 0] [i1, 3] [i2, 4]   acc = phi [0, 0] [acc, 3] [acc2, 4]
///      c = i < bound                    -> 2 | 5
///   2: odd = i & 1 ; z = odd == 0       -> 3 | 4
///   3: i1 = i + 1                       -> 1        (continue)
///   4: acc2 = acc + i ; i2 = i + 2      -> 1
///   5: ret acc
/// ```
fn two_latches() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(30), 3), (v(41), 4)]),
                phi(20, &[(k(0), 0), (v(20), 3), (v(40), 4)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 5),
        ),
        blk(
            2,
            vec![
                bin(21, IrBinOp::And, v(10), k(1)),
                cmp(22, IrCmpOp::Eq, v(21), k(0)),
            ],
            cbr(v(22), 3, 4),
        ),
        blk(3, vec![add(30, v(10), k(1))], br(1)),
        blk(4, vec![add(40, v(20), v(10)), add(41, v(10), k(2))], br(1)),
        blk(5, vec![], ret(v(20))),
    ])
}

#[test]
fn several_back_edges_share_one_bottom_test() {
    let after = rotated("two latches", &two_latches, 1);
    let tests: Vec<u32> = after
        .blocks
        .iter()
        .filter(|b| {
            b.label != BlockId(1)
                && b.label != BlockId(2)
                && matches!(b.terminator, Terminator::CondBranch { .. })
        })
        .map(|b| b.label.0)
        .collect();
    assert_eq!(
        tests.len(),
        1,
        "one new latch holds the bottom test: {tests:?}"
    );
    assert!(tests[0] > 5, "and it is a new block");
}

/// A back edge that is one arm of a conditional branch: the edge cannot
/// carry the test, a new latch does.
///
/// ```text
///   1: i = phi [seed, 0] [i', 2] [i', 3]   acc = phi [0, 0] [acc1, 2] [acc2, 3]
///      c = i < bound                    -> 2 | 4
///   2: acc1 = acc + i ; i' = i + 1 ; m = i' & 3 ; z = m != 0   -> 1 | 3
///   3: acc2 = acc1 * 2                  -> 1
///   4: ret acc
/// ```
fn conditional_back_edge() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(21), 2), (v(21), 3)]),
                phi(20, &[(k(0), 0), (v(22), 2), (v(30), 3)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 4),
        ),
        blk(
            2,
            vec![
                add(22, v(20), v(10)),
                add(21, v(10), k(1)),
                bin(23, IrBinOp::And, v(21), k(3)),
                cmp(24, IrCmpOp::Ne, v(23), k(0)),
            ],
            cbr(v(24), 1, 3),
        ),
        blk(3, vec![mul(30, v(22), k(2))], br(1)),
        blk(4, vec![], ret(v(20))),
    ])
}

#[test]
fn a_conditional_back_edge_gets_a_latch_of_its_own() {
    rotated("conditional back edge", &conditional_back_edge, 1);
}

// ── Exits out of the body ───────────────────────────────────────────────────

/// A `break` into the block the header leaves to: that block has a phi of
/// its own, and gains an edge from the latch.
///
/// ```text
///   1: i = phi [seed, 0] [i', 3]   acc = phi [0, 0] [acc', 3]
///      t = i * 2
///      c = i < bound                    -> 2 | 4
///   2: big = acc > extra                -> 4 | 3     (break)
///   3: acc' = acc + t ; i' = i + 1      -> 1
///   4: r = phi [acc, 1] [i, 2] ; s = r + t ; ret s
/// ```
fn break_into_the_exit() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(31), 3)]),
                phi(20, &[(k(0), 0), (v(30), 3)]),
                mul(11, v(10), k(2)),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 4),
        ),
        blk(
            2,
            vec![cmp(21, IrCmpOp::Sgt, v(20), v(3))],
            cbr(v(21), 4, 3),
        ),
        blk(3, vec![add(30, v(20), v(11)), add(31, v(10), k(1))], br(1)),
        blk(
            4,
            vec![phi(40, &[(v(20), 1), (v(10), 2)]), add(41, v(40), v(11))],
            ret(v(41)),
        ),
    ])
}

#[test]
fn an_exit_block_other_blocks_reach_keeps_every_edge_served() {
    let after = rotated("break into the exit", &break_into_the_exit, 1);
    let exit = after.blocks.iter().find(|b| b.label == BlockId(4)).unwrap();
    for inst in &exit.instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            assert_eq!(incoming.len(), 3, "guard, break and latch: {inst:?}");
        }
    }
}

/// A `break` to a block of its own that later joins the regular exit, with
/// header values read behind the join: the merge is needed at the join, two
/// blocks behind the loop.
///
/// ```text
///   1: i = phi [seed, 0] [i', 3]   acc = phi [0, 0] [acc', 3]
///      t = i + 100
///      c = i < bound                    -> 2 | 4
///   2: big = acc > extra                -> 5 | 3     (break)
///   3: acc' = acc + i ; i' = i + 1      -> 1
///   4:                                  -> 6
///   5:                                  -> 6
///   6: r = acc * 3 ; s = r + t ; u = s + i ; ret u
/// ```
fn break_to_a_block_of_its_own() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(31), 3)]),
                phi(20, &[(k(0), 0), (v(30), 3)]),
                add(11, v(10), k(100)),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 4),
        ),
        blk(
            2,
            vec![cmp(21, IrCmpOp::Sgt, v(20), v(3))],
            cbr(v(21), 5, 3),
        ),
        blk(3, vec![add(30, v(20), v(10)), add(31, v(10), k(1))], br(1)),
        blk(4, vec![], br(6)),
        blk(5, vec![], br(6)),
        blk(
            6,
            vec![
                mul(60, v(20), k(3)),
                add(61, v(60), v(11)),
                add(62, v(61), v(10)),
            ],
            ret(v(62)),
        ),
    ])
}

#[test]
fn readers_two_blocks_behind_the_loop_get_their_merge() {
    let after = rotated(
        "break to a block of its own",
        &break_to_a_block_of_its_own,
        1,
    );
    let join = after.blocks.iter().find(|b| b.label == BlockId(6)).unwrap();
    let phis = join
        .instructions
        .iter()
        .filter(|i| matches!(i, Instruction::Phi { .. }))
        .count();
    assert_eq!(phis, 3, "i, acc and t each merge at the join: {join:?}");
    // The block behind the break needs none: only the loop reaches it.
    let behind_break = after.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
    assert!(behind_break.instructions.is_empty());
}

// ── Loops in loops ──────────────────────────────────────────────────────────

/// ```text
///   1: i = phi [seed, 0] [i', 5]   acc = phi [0, 0] [a, 5]
///      c = i < bound                    -> 2 | 6
///   2:                                  -> 3
///   3: j = phi [0, 2] [j', 4]   a = phi [acc, 2] [a', 4]
///      d = j < extra                    -> 4 | 5
///   4: ij = i * j ; a' = a + ij ; j' = j + 1   -> 3
///   5: i' = i + 1                       -> 1
///   6: ret acc
/// ```
fn nest() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(50), 5)]),
                phi(20, &[(k(0), 0), (v(31), 5)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 6),
        ),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![
                phi(30, &[(k(0), 2), (v(42), 4)]),
                phi(31, &[(v(20), 2), (v(41), 4)]),
                cmp(32, IrCmpOp::Slt, v(30), v(3)),
            ],
            cbr(v(32), 4, 5),
        ),
        blk(
            4,
            vec![
                mul(40, v(10), v(30)),
                add(41, v(31), v(40)),
                add(42, v(30), k(1)),
            ],
            br(3),
        ),
        blk(5, vec![add(50, v(10), k(1))], br(1)),
        blk(6, vec![], ret(v(20))),
    ])
}

#[test]
fn both_loops_of_a_nest_rotate() {
    rotated("nest", &nest, 2);
}

/// The body's first block is the header of an inner loop that is
/// bottom-tested already: the inner loop's back edge reaches it too, so it
/// cannot be the outer loop's header and an empty block in front of it is.
///
/// ```text
///   1: i = phi [seed, 0] [i', 3]   acc = phi [0, 0] [s', 3]
///      c = i < bound                    -> 2 | 4
///   2: s = phi [acc, 1] [s', 2]   n = phi [0, 1] [n', 2]
///      s' = s + i ; n' = n + 1 ; d = n' < extra   -> 2 | 3
///   3: i' = i + 1                       -> 1
///   4: ret acc
/// ```
fn inner_loop_at_the_top_of_the_body() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(30), 3)]),
                phi(20, &[(k(0), 0), (v(22), 3)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 4),
        ),
        blk(
            2,
            vec![
                phi(21, &[(v(20), 1), (v(22), 2)]),
                phi(25, &[(k(0), 1), (v(23), 2)]),
                add(22, v(21), v(10)),
                add(23, v(25), k(1)),
                cmp(24, IrCmpOp::Slt, v(23), v(3)),
            ],
            cbr(v(24), 2, 3),
        ),
        blk(3, vec![add(30, v(10), k(1))], br(1)),
        blk(4, vec![], ret(v(20))),
    ])
}

#[test]
fn a_body_that_starts_with_an_inner_loop_gets_a_header_of_its_own() {
    let after = rotated("inner loop first", &inner_loop_at_the_top_of_the_body, 1);
    // The inner loop's header still has exactly its two edges.
    let inner = after.blocks.iter().find(|b| b.label == BlockId(2)).unwrap();
    for inst in &inner.instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            assert_eq!(incoming.len(), 2, "{inst:?}");
            assert!(
                incoming.iter().all(|(_, from)| *from != BlockId(1)),
                "{inst:?}"
            );
        }
    }
}

// ── A header entered over several edges ─────────────────────────────────────

/// ```text
///   0: z = seed < 0                     -> 7 | 8
///   7:                                  -> 1
///   8:                                  -> 1
///   1: i = phi [seed, 7] [extra, 8] [i', 2]   acc = phi [1, 7] [2, 8] [acc', 2]
///      c = i < bound                    -> 2 | 3
///   2: acc' = acc + i ; i' = i + 1      -> 1
///   3: ret acc
/// ```
fn two_entries() -> IrFunction {
    func(vec![
        blk(
            0,
            {
                let mut insts = params();
                insts.push(cmp(4, IrCmpOp::Slt, v(1), k(0)));
                insts
            },
            cbr(v(4), 7, 8),
        ),
        blk(7, vec![], br(1)),
        blk(8, vec![], br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 7), (v(3), 8), (v(21), 2)]),
                phi(20, &[(k(1), 7), (k(2), 8), (v(20), 2)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(2, vec![add(22, v(20), v(10)), add(21, v(10), k(1))], br(1)),
        blk(3, vec![], ret(v(20))),
    ])
}

#[test]
fn a_header_with_several_entries_stays_their_join() {
    let after = rotated("two entries", &two_entries, 1);
    let guard = after.blocks.iter().find(|b| b.label == BlockId(1)).unwrap();
    for inst in &guard.instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            assert_eq!(
                incoming.len(),
                2,
                "the two entries, not the latch: {inst:?}"
            );
        }
    }
    // `acc` never changes in the loop: its merge joined one value with
    // itself and is gone, which leaves the guard's two phis and the loop's
    // one for `i`.
    assert_eq!(count_phis(&after), 3, "{after:#?}");
}

// ── Refusals ────────────────────────────────────────────────────────────────

/// A refused loop is left as it was, value numbers and block order included.
fn refused(what: &str, mut f: IrFunction) {
    let before = format!("{f:?}");
    assert_eq!(rotate_every_loop(&mut f), 0, "{what}: rotated");
    assert_eq!(
        format!("{f:?}"),
        before,
        "{what}: a refusal changed the function"
    );
}

#[test]
fn a_bottom_tested_loop_is_left_alone() {
    // 1: i = phi ; i' = i + 1 ; c = i' < bound -> 1 | 2
    refused(
        "self loop",
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(1), 0), (v(11), 1)]),
                    add(11, v(10), k(1)),
                    cmp(12, IrCmpOp::Slt, v(11), v(2)),
                ],
                cbr(v(12), 1, 2),
            ),
            blk(2, vec![], ret(v(11))),
        ]),
    );
    // A body that starts with an exit test and ends with one: the latch
    // leaves the loop, so the loop is rotated already.
    refused(
        "exit at the top of a bottom-tested loop",
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(1), 0), (v(21), 2)]),
                    cmp(12, IrCmpOp::Slt, v(10), v(2)),
                ],
                cbr(v(12), 2, 3),
            ),
            blk(
                2,
                vec![add(21, v(10), k(1)), cmp(22, IrCmpOp::Slt, v(21), v(3))],
                cbr(v(22), 1, 3),
            ),
            blk(3, vec![], ret(k(0))),
        ]),
    );
}

#[test]
fn a_header_that_does_more_than_test_is_left_alone() {
    let with = |inst: Instruction| {
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(1), 0), (v(20), 2)]),
                    inst,
                    cmp(12, IrCmpOp::Slt, v(10), v(2)),
                ],
                cbr(v(12), 2, 3),
            ),
            blk(2, vec![add(20, v(10), k(1))], br(1)),
            blk(3, vec![], ret(v(10))),
        ])
    };
    refused("a store in the header", with(store(v(10), 3)));
    refused(
        "a volatile load in the header",
        with(Instruction::Load {
            dest: Value(11),
            ptr: Value(3),
            ty: Ty::I64,
            seg_override: AddressSpace::Default,
            volatile: true,
        }),
    );
    refused(
        "an alloca in the header",
        with(Instruction::Alloca {
            dest: Value(11),
            ty: Ty::I64,
            size: 8,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        }),
    );
}

#[test]
fn a_header_too_large_to_copy_is_left_alone() {
    let build = |extra: usize| {
        let mut header = vec![phi(10, &[(v(1), 0), (v(20), 2)])];
        let mut last = 10;
        for n in 0..extra {
            let dest = 100 + n as u32;
            header.push(add(dest, v(last), k(1)));
            last = dest;
        }
        header.push(cmp(12, IrCmpOp::Slt, v(last), v(2)));
        func(vec![
            blk(0, params(), br(1)),
            blk(1, header, cbr(v(12), 2, 3)),
            blk(2, vec![add(20, v(10), k(1))], br(1)),
            blk(3, vec![], ret(v(10))),
        ])
    };
    // The compare counts: MAX_HEADER_INSTS instructions are copied, one
    // more is not.
    rotated("a header at the limit", &|| build(MAX_HEADER_INSTS - 1), 1);
    refused("a header past the limit", build(MAX_HEADER_INSTS));
}

#[test]
fn a_comparison_read_outside_the_header_is_left_alone() {
    // The body returns the header's comparison result. A merge of it and of
    // its copy would need a type, and a comparison states none.
    refused(
        "comparison read in the body",
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(1), 0), (v(20), 2)]),
                    cmp(12, IrCmpOp::Slt, v(10), v(2)),
                ],
                cbr(v(12), 2, 3),
            ),
            blk(2, vec![add(20, v(10), v(12))], br(1)),
            blk(3, vec![], ret(v(10))),
        ]),
    );
    // Read by nothing but the header's own branch, it is no obstacle.
    rotated(
        "comparison read by the branch",
        &|| counted(IrCmpOp::Slt, false, false, v(10)),
        1,
    );
}

#[test]
fn a_loop_whose_header_is_the_entry_block_is_left_alone() {
    refused(
        "header is block 0",
        func(vec![
            blk(
                0,
                vec![phi(10, &[(v(20), 1)]), cmp(12, IrCmpOp::Slt, v(10), k(5))],
                cbr(v(12), 1, 2),
            ),
            blk(1, vec![add(20, v(10), k(1))], br(0)),
            blk(2, vec![], ret(k(0))),
        ]),
    );
}

#[test]
fn a_constant_cannot_replace_a_phi_the_test_dereferences() {
    // 1: p = phi [extra, 0] [0, 2] ; x = load p ; c = x < bound
    // The latch would have to load through the constant.
    refused(
        "load through a phi reset to a constant",
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(3), 0), (k(0), 2)]),
                    load(11, 10),
                    cmp(12, IrCmpOp::Slt, v(11), v(2)),
                ],
                cbr(v(12), 2, 3),
            ),
            blk(2, vec![], br(1)),
            blk(3, vec![], ret(k(0))),
        ]),
    );
}

// ── The profit policy ───────────────────────────────────────────────────────

/// A stand-in for LICM: move instruction `dest` into the preheader of the
/// loop it is in, when that loop has one that always enters it (a block
/// that ends in a plain branch to the loop's header and is the only way
/// in). That is the place rotation makes; a top-tested loop has none
/// behind its test.
fn hoist_behind_a_guard(f: &mut IrFunction, dest: u32) -> usize {
    let (cfg, loops) = loops_innermost_first(f);
    let Some(at) = f
        .blocks
        .iter()
        .position(|b| b.instructions.iter().any(|i| i.dest() == Some(Value(dest))))
    else {
        return 0;
    };
    let Some(lp) = loops.iter().find(|lp| lp.body.contains(&at)) else {
        return 0;
    };
    let entries: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p))
        .collect();
    let &[preheader] = entries.as_slice() else {
        return 0;
    };
    // Only out of a block that runs on every trip: the loop's header.
    if at != lp.header || !matches!(f.blocks[preheader].terminator, Terminator::Branch(_)) {
        return 0;
    }
    let index = f.blocks[at]
        .instructions
        .iter()
        .position(|i| i.dest() == Some(Value(dest)))
        .unwrap();
    let inst = f.blocks[at].instructions.remove(index);
    f.blocks[preheader].instructions.push(inst);
    1
}

/// ```text
///   1: i = phi [seed, 0] [i', 2]   acc = phi [0, 0] [acc', 2]
///      c = i < bound                    -> 2 | 3
///   2: x = load extra ; acc' = acc + x ; i' = i + 1   -> 1
///   3: ret acc
/// ```
/// The load does not depend on the loop, and must not run when the loop
/// does not.
fn invariant_load_in_the_body() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(22), 2)]),
                phi(20, &[(k(0), 0), (v(21), 2)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(
            2,
            vec![load(23, 3), add(21, v(20), v(23)), add(22, v(10), k(1))],
            br(1),
        ),
        blk(3, vec![], ret(v(20))),
    ])
}

/// A stand-in for a hoist that needs no guard: move instruction `dest`
/// into the block in front of the loop it is in, whatever the loop looks
/// like.
fn hoist_anyway(f: &mut IrFunction, dest: u32) -> usize {
    let (cfg, loops) = loops_innermost_first(f);
    let Some(at) = f
        .blocks
        .iter()
        .position(|b| b.instructions.iter().any(|i| i.dest() == Some(Value(dest))))
    else {
        return 0;
    };
    let Some(lp) = loops.iter().find(|lp| lp.body.contains(&at)) else {
        return 0;
    };
    let Some(outside) = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .find(|p| !lp.body.contains(p))
    else {
        return 0;
    };
    let index = f.blocks[at]
        .instructions
        .iter()
        .position(|i| i.dest() == Some(Value(dest)))
        .unwrap();
    let inst = f.blocks[at].instructions.remove(index);
    f.blocks[outside].instructions.push(inst);
    1
}

#[test]
fn a_loop_is_rotated_when_that_lets_something_leave_it() {
    let before = invariant_load_in_the_body();
    let mut f = invariant_load_in_the_body();
    // Nothing can be hoisted out of the top-tested loop.
    assert_eq!(hoist_behind_a_guard(&mut f, 23), 0);
    let mut calls = 0;
    let done = rotate_for_hoisting(&mut f, |f| {
        calls += 1;
        hoist_behind_a_guard(f, 23)
    });
    // The trial, and the copy that is only hoisted (asked once the load
    // was seen to leave: would it have left anyway?).
    assert_eq!((done, calls), (Hoisted { loops: 1, moved: 1 }, 2));
    assert_eq!(violations(&f), Vec::<String>::new());
    // The load sits in the preheader, behind the guard.
    let guard = f.blocks.iter().position(|b| b.label == BlockId(1)).unwrap();
    assert!(
        matches!(
            f.blocks[guard + 1].instructions[..],
            [Instruction::Load { .. }]
        ),
        "{:?}",
        f.blocks[guard + 1]
    );
    for input in grid() {
        assert_eq!(run(&f, &input), run(&before, &input), "on {input:?}");
    }
}

#[test]
fn a_loop_nothing_leaves_is_not_rotated() {
    let mut f = invariant_load_in_the_body();
    let before = format!("{f:?}");
    let mut calls = 0;
    let done = rotate_for_hoisting(&mut f, |_| {
        calls += 1;
        0
    });
    assert_eq!(
        (done, calls),
        (Hoisted::default(), 1),
        "one trial, and its result thrown away"
    );
    assert_eq!(
        format!("{f:?}"),
        before,
        "the function is exactly as it was"
    );
}

#[test]
fn a_loop_without_a_candidate_is_not_even_tried() {
    let mut f = counted(IrCmpOp::Slt, false, false, v(10));
    let before = format!("{f:?}");
    let done = rotate_for_hoisting(&mut f, |_| panic!("nothing to try"));
    assert_eq!(done, Hoisted::default());
    assert_eq!(format!("{f:?}"), before);
}

/// What moves out of the top-tested loop just as well is not the
/// rotation's doing: the passes between two hoists leave such instructions
/// behind, and a loop is not rotated for them.
#[test]
fn what_leaves_without_the_rotation_does_not_pay_for_it() {
    let mut f = invariant_load_in_the_body();
    let before = format!("{f:?}");
    let mut calls = 0;
    let done = rotate_for_hoisting(&mut f, |f| {
        calls += 1;
        hoist_anyway(f, 23)
    });
    assert_eq!(
        (done, calls),
        (Hoisted::default(), 2),
        "the trial and the copy that is only hoisted"
    );
    assert_eq!(
        format!("{f:?}"),
        before,
        "the function is exactly as it was"
    );
}

/// Two loops in a row, each with a load in its body.
fn two_loops_with_loads() -> IrFunction {
    func(vec![
        blk(0, params(), br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(22), 2)]),
                phi(20, &[(k(0), 0), (v(21), 2)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(
            2,
            vec![load(23, 3), add(21, v(20), v(23)), add(22, v(10), k(1))],
            br(1),
        ),
        blk(3, vec![], br(4)),
        blk(
            4,
            vec![
                phi(40, &[(v(1), 3), (v(52), 5)]),
                phi(41, &[(v(20), 3), (v(51), 5)]),
                cmp(42, IrCmpOp::Slt, v(40), v(2)),
            ],
            cbr(v(42), 5, 6),
        ),
        blk(
            5,
            vec![load(53, 3), add(51, v(41), v(53)), add(52, v(40), k(1))],
            br(4),
        ),
        blk(6, vec![], ret(v(41))),
    ])
}

#[test]
fn only_the_loops_that_pay_are_rotated() {
    let before = two_loops_with_loads();
    let mut f = two_loops_with_loads();
    let mut calls = 0;
    // The stand-in can move the second loop's load and not the first's.
    let done = rotate_for_hoisting(&mut f, |f| {
        calls += 1;
        hoist_behind_a_guard(f, 53)
    });
    // First attempt: both rotated, one paid (and the copy that is only
    // hoisted was asked about it). Second: the one that paid, alone, and
    // that is the function.
    assert_eq!((done, calls), (Hoisted { loops: 1, moved: 1 }, 3));
    assert_eq!(violations(&f), Vec::<String>::new());
    // The first loop is exactly as it was.
    for label in [1, 2, 3] {
        let was = before
            .blocks
            .iter()
            .find(|b| b.label == BlockId(label))
            .unwrap();
        let is = f.blocks.iter().find(|b| b.label == BlockId(label)).unwrap();
        assert_eq!(format!("{is:?}"), format!("{was:?}"), ".LBB{label}");
    }
    for input in grid() {
        assert_eq!(run(&f, &input), run(&before, &input), "on {input:?}");
    }
}

// ── What a rotation has to be worth ─────────────────────────────────────────

/// [`nest`] with a load of `extra` in the outer loop's latch (block 5): it
/// runs once per trip of the outer loop, which runs the inner loop.
fn load_in_the_outer_loop_of_a_nest() -> IrFunction {
    let mut f = nest();
    f.blocks[5].instructions = vec![load(51, 3), add(52, v(10), v(51)), add(50, v(52), k(1))];
    f
}

#[test]
fn a_load_saved_once_per_trip_of_an_outer_loop_rotates_nothing() {
    let mut f = load_in_the_outer_loop_of_a_nest();
    assert_eq!(violations(&f), Vec::<String>::new(), "the fixture");
    let before = format!("{f:?}");
    let mut moved = 0;
    let done = rotate_for_hoisting(&mut f, |f| {
        let n = hoist_behind_a_guard_from(f, 51, 5);
        moved += n;
        n
    });
    assert!(moved > 0, "the stand-in did hoist the load in the trial");
    assert_eq!(done, Hoisted::default());
    assert_eq!(
        format!("{f:?}"),
        before,
        "the function is exactly as it was"
    );
}

/// [`hoist_behind_a_guard`] for an instruction in block `home` of a loop of
/// several blocks: into the preheader, once the loop has one that always
/// enters it.
fn hoist_behind_a_guard_from(f: &mut IrFunction, dest: u32, home: u32) -> usize {
    let (cfg, loops) = loops_innermost_first(f);
    let Some(at) = f.blocks.iter().position(|b| b.label == BlockId(home)) else {
        return 0;
    };
    let Some(index) = f.blocks[at]
        .instructions
        .iter()
        .position(|i| i.dest() == Some(Value(dest)))
    else {
        return 0;
    };
    // The outermost loop the block is in.
    let Some(lp) = loops.iter().rev().find(|lp| lp.body.contains(&at)) else {
        return 0;
    };
    let entries: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p))
        .collect();
    let &[preheader] = entries.as_slice() else {
        return 0;
    };
    // Behind a guard only: the preheader's own way in is a test.
    let ways_in = cfg.preds.row(preheader);
    let guarded = !ways_in.is_empty()
        && ways_in.iter().all(|&p| {
            matches!(
                f.blocks[p as usize].terminator,
                Terminator::CondBranch { .. }
            )
        });
    if !guarded || !matches!(f.blocks[preheader].terminator, Terminator::Branch(_)) {
        return 0;
    }
    let inst = f.blocks[at].instructions.remove(index);
    f.blocks[preheader].instructions.push(inst);
    1
}

/// The same nest with a division in the outer latch: worth rotating the
/// nest for, where the load was not.
#[test]
fn a_division_saved_once_per_trip_of_an_outer_loop_does_pay() {
    let build = || {
        let mut f = nest();
        // d = extra + 7 is never zero on the grid.
        f.blocks[0].instructions.push(add(4, v(3), k(7)));
        // q = bound / d ; acc = a + q ; i' = i + 1
        f.blocks[5].instructions = vec![
            bin(51, IrBinOp::SDiv, v(2), v(4)),
            add(54, v(31), v(51)),
            add(50, v(10), k(1)),
        ];
        f.blocks[1].instructions[1] = phi(20, &[(k(0), 0), (v(54), 5)]);
        f
    };
    let before = build();
    assert_eq!(violations(&before), Vec::<String>::new(), "the fixture");
    let mut f = build();
    let done = rotate_for_hoisting(&mut f, |f| hoist_behind_a_guard_from(f, 51, 5));
    assert_eq!(done, Hoisted { loops: 1, moved: 1 });
    assert_eq!(violations(&f), Vec::<String>::new());
    for input in grid() {
        assert_eq!(run(&f, &input), run(&before, &input), "on {input:?}");
    }
}

#[test]
fn what_left_a_loop_is_counted_by_kind() {
    let call = |dest: u32, pure: bool| Instruction::Call {
        func: "f".into(),
        info: crate::ir::reexports::CallInfo {
            dest: Some(Value(dest)),
            is_pure: pure,
            ..Default::default()
        },
    };
    let volatile = Instruction::Load {
        dest: Value(91),
        ptr: Value(3),
        ty: Ty::I64,
        seg_override: AddressSpace::Default,
        volatile: true,
    };
    let f = func(vec![
        blk(
            0,
            vec![
                load(10, 3),
                load(11, 3),
                volatile,
                bin(12, IrBinOp::SDiv, v(10), v(11)),
                bin(13, IrBinOp::URem, v(10), v(11)),
                add(14, v(10), v(11)),
                call(90, true),
                call(92, false),
                store(v(14), 3),
            ],
            br(1),
        ),
        blk(
            1,
            vec![load(20, 3), store(v(20), 3), store(v(20), 3)],
            ret(k(0)),
        ),
    ]);
    let first = Guarded::of(&f, |label| label == BlockId(0));
    assert_eq!(
        first.loads,
        [10, 11].into_iter().collect(),
        "a volatile load never moves"
    );
    assert_eq!(first.divisions, [12, 13].into_iter().collect());
    assert_eq!(
        first.calls,
        [90].into_iter().collect(),
        "a call that writes never moves"
    );
    assert!(!first.is_empty());
    assert!(Guarded::of(&f, |_| false).is_empty());

    // Every load of the function is still somewhere: what leaves the loop
    // was hoisted.
    let everywhere = loads_of(&f);
    assert_eq!(everywhere, [10, 11, 20].into_iter().collect());
    let mut after = first.clone();
    assert!(first.not_in(&after).is_empty());
    after.loads.remove(&11);
    // The copy of a header instruction in the latch has a new number and
    // is nobody's loss; neither is a load a pass put into the loop.
    after.loads.insert(77);
    after.divisions.insert(78);
    let gone = first.not_in(&after);
    assert_eq!(gone.loads, [11].into_iter().collect());
    assert!(gone.divisions.is_empty() && gone.calls.is_empty());
    let left = Left::of(&gone, &everywhere);
    assert_eq!(
        (
            left.hoisted.as_slice(),
            left.carried,
            left.divisions,
            left.calls
        ),
        (&[11][..], 0, 0, 0)
    );
    assert!(left.is_hoisted_loads_only());
    assert_eq!(
        (left.worth(), left.to_string()),
        (LOAD_WORTH, "1 load".to_string())
    );

    // A load that is nowhere afterwards was replaced by a carried value.
    let carried = Left::of(&gone, &[10, 20].into_iter().collect());
    assert_eq!((carried.hoisted.as_slice(), carried.carried), (&[][..], 1));
    assert!(!carried.is_hoisted_loads_only());
    assert_eq!(
        (carried.worth(), carried.to_string()),
        (CARRIED_WORTH, "1 carried load".to_string())
    );

    let everything = Left::of(
        &first.not_in(&Guarded::default()),
        &[10].into_iter().collect(),
    );
    assert!(!everything.is_hoisted_loads_only());
    assert_eq!(
        everything.worth(),
        LOAD_WORTH + CARRIED_WORTH + 3 * HEAVY_WORTH
    );
    assert_eq!(
        everything.to_string(),
        "1 load, 1 carried load, 2 divisions, 1 call"
    );

    // What also left without the rotation is not credited: of the two
    // loads that are gone, the unrotated function kept only one.
    let stayed = Guarded {
        loads: [10].into_iter().collect(),
        ..Guarded::default()
    };
    let waited = first.not_in(&Guarded::default()).also_in(&stayed);
    assert_eq!(waited.loads, [10].into_iter().collect());
    assert!(waited.divisions.is_empty() && waited.calls.is_empty());
    assert_eq!(Left::of(&waited, &everywhere).worth(), LOAD_WORTH);
}

#[test]
fn a_trip_counts_ten_of_every_loop_inside_it() {
    let f = nest();
    let (_, loops) = loops_innermost_first(&f);
    let depth = loop_depths(f.blocks.len(), &loops);
    assert_eq!(depth, [0, 1, 1, 2, 2, 1, 0]);
    let all = |labels: &[u32]| labels.iter().map(|&l| BlockId(l)).collect::<FxHashSet<_>>();
    // The inner loop alone: its test and its three instructions.
    assert_eq!(trip_size(&f, &all(&[3, 4]), &depth), 1 + 3);
    // The nest: the outer test and increment once, the inner loop ten times.
    assert_eq!(
        trip_size(&f, &all(&[1, 2, 3, 4, 5]), &depth),
        1 + 1 + 10 * (1 + 3)
    );
    assert_eq!(trip_size(&f, &all(&[]), &depth), 0);
}

/// `p` (v3) is the address the hoisted load (v9, in block 1) read. Blocks:
/// 0 entry, 1 preheader, 2 the loop, 3 behind it.
fn hoisted_load(ty: Ty, in_loop: Vec<Instruction>, behind: Vec<Instruction>) -> IrFunction {
    let mut body = vec![phi(10, &[(k(0), 1), (v(11), 2)])];
    body.extend(in_loop);
    body.extend([add(11, v(10), v(9)), cmp(12, IrCmpOp::Slt, v(11), v(2))]);
    func(vec![
        blk(0, params(), cbr(v(1), 1, 3)),
        blk(
            1,
            vec![Instruction::Load {
                dest: Value(9),
                ptr: Value(3),
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            }],
            br(2),
        ),
        blk(2, body, cbr(v(12), 2, 3)),
        blk(3, behind, ret(k(0))),
    ])
}

#[test]
fn a_hoisted_load_is_free_when_it_takes_over_its_address_register() {
    let free = |f: &IrFunction| loads_hold_no_more(f, BlockId(2), &[9]);
    assert!(
        free(&hoisted_load(Ty::I64, vec![], vec![])),
        "the address is read nowhere from the loop on"
    );
    assert!(free(&hoisted_load(Ty::I32, vec![], vec![])));
    assert!(free(&hoisted_load(Ty::Ptr, vec![], vec![])));
    // The address stays live: the value is a register more.
    assert!(
        !free(&hoisted_load(Ty::I64, vec![load(13, 3)], vec![])),
        "read in the loop"
    );
    assert!(
        !free(&hoisted_load(Ty::I64, vec![], vec![store(k(1), 3)])),
        "read behind the loop"
    );
    let mut merged = hoisted_load(Ty::I64, vec![], vec![]);
    merged.blocks[3].instructions = vec![phi(14, &[(k(0), 0), (v(3), 2)])];
    assert!(!free(&merged), "handed over the loop's exit edge");
    // A phi behind the loop that takes the address over the edge around
    // the loop does not keep it alive in the loop.
    let mut around = hoisted_load(Ty::I64, vec![], vec![]);
    around.blocks[3].instructions = vec![phi(14, &[(v(3), 0), (k(0), 2)])];
    assert!(free(&around));
    // A floating-point value does not fit the register its address had.
    assert!(!free(&hoisted_load(Ty::F64, vec![], vec![])));
    assert!(!free(&hoisted_load(Ty::F32, vec![], vec![])));
    // A load that is nowhere was not hoisted.
    assert!(!loads_hold_no_more(
        &hoisted_load(Ty::I64, vec![], vec![]),
        BlockId(2),
        &[9, 99]
    ));
    // Two loads through one address free one register and take two.
    let mut two = hoisted_load(Ty::I64, vec![], vec![]);
    two.blocks[1].instructions.push(load(8, 3));
    assert!(!loads_hold_no_more(&two, BlockId(2), &[8, 9]));
}

/// The loop sits inside another one that computes the address anew each
/// trip (`q = base + i`, in block 1) and writes through it before the
/// inner loop. From the inner loop on, `q` is read again only behind its
/// next definition: this trip's `q` is dead and the hoisted value takes its
/// register.
/// ```text
///   0: params                                        -> 1
///   1: i = phi [seed, 0] [i', 4] ; q = extra + i
///      store 7 -> q ; c = i < bound                  -> 2 | 5
///   2: x = load q          (the inner loop's preheader)   -> 3
///   3: j = phi [0, 2] [j', 3] ; j' = j + x ; d = j' < bound   -> 3 | 4
///   4: i' = i + 1                                    -> 1
///   5: ret 0
/// ```
#[test]
fn an_address_computed_anew_by_an_enclosing_loop_is_not_live_around_it() {
    let nested = |behind_inner: Vec<Instruction>| {
        let mut latch = behind_inner;
        latch.push(add(41, v(10), k(1)));
        func(vec![
            blk(0, params(), br(1)),
            blk(
                1,
                vec![
                    phi(10, &[(v(1), 0), (v(41), 4)]),
                    add(11, v(3), v(10)),
                    store(k(7), 11),
                    cmp(12, IrCmpOp::Slt, v(10), v(2)),
                ],
                cbr(v(12), 2, 5),
            ),
            blk(2, vec![load(20, 11)], br(3)),
            blk(
                3,
                vec![
                    phi(30, &[(k(0), 2), (v(31), 3)]),
                    add(31, v(30), v(20)),
                    cmp(32, IrCmpOp::Slt, v(31), v(2)),
                ],
                cbr(v(32), 3, 4),
            ),
            blk(4, latch, br(1)),
            blk(5, vec![], ret(k(0))),
        ])
    };
    let f = nested(vec![]);
    assert_eq!(violations(&f), Vec::<String>::new(), "the fixture");
    assert!(loads_hold_no_more(&f, BlockId(3), &[20]));
    // Read behind the inner loop and in front of the next definition: live.
    assert!(!loads_hold_no_more(
        &nested(vec![load(40, 11)]),
        BlockId(3),
        &[20]
    ));
}

#[test]
fn a_load_that_costs_a_register_rotates_nothing() {
    // The address is read again behind the loop.
    let build = || {
        let mut f = invariant_load_in_the_body();
        f.blocks[3] = blk(3, vec![load(30, 3), add(31, v(20), v(30))], ret(v(31)));
        f
    };
    let mut f = build();
    let before = format!("{f:?}");
    let done = rotate_for_hoisting(&mut f, |f| hoist_behind_a_guard(f, 23));
    assert_eq!(done, Hoisted::default());
    assert_eq!(format!("{f:?}"), before);
}

// ── Memory promotion behind the guard ───────────────────────────────────────

/// `for (i = seed; i < bound; i++) *p += i;` and the cell read behind the
/// loop, with `p` = `extra`:
/// ```text
///   0: params ; store 100 -> p              -> 1
///   1: i = phi [seed, 0] [i', 2]
///      c = i < bound                        -> 2 | 3
///   2: x = load p ; y = x + i ; store y -> p ; i' = i + 1   -> 1
///   3: r = load p ; ret r
/// ```
fn accumulator_behind_a_pointer() -> IrFunction {
    let mut entry = params();
    entry.push(store(k(100), 3));
    func(vec![
        blk(0, entry, br(1)),
        blk(
            1,
            vec![
                phi(10, &[(v(1), 0), (v(22), 2)]),
                cmp(12, IrCmpOp::Slt, v(10), v(2)),
            ],
            cbr(v(12), 2, 3),
        ),
        blk(
            2,
            vec![
                load(20, 3),
                add(21, v(20), v(10)),
                store(v(21), 3),
                add(22, v(10), k(1)),
            ],
            br(1),
        ),
        blk(3, vec![load(30, 3)], ret(v(30))),
    ])
}

#[test]
fn a_memory_recurrence_is_promoted_behind_the_guard() {
    use crate::passes::loop_memory_promote;
    let before = accumulator_behind_a_pointer();
    assert_eq!(violations(&before), Vec::<String>::new(), "the fixture");
    // The top-tested loop is not promotable: nothing may touch `*p` when
    // the loop does not run, and a parameter cannot be proven to point at
    // memory that may be read early or written late.
    let mut top_tested = accumulator_behind_a_pointer();
    top_tested.next_value_id = 100;
    assert_eq!(loop_memory_promote::run(&mut top_tested), 0);

    let mut f = accumulator_behind_a_pointer();
    f.next_value_id = 100;
    let done = rotate_for_hoisting(&mut f, loop_memory_promote::run);
    assert_eq!(done, Hoisted { loops: 1, moved: 1 });
    assert_eq!(violations(&f), Vec::<String>::new());

    // The loop touches memory no more: one load in front of it, behind the
    // guard, and one store on its exit edge.
    let (_, loops) = loops_innermost_first(&f);
    let [lp] = loops.as_slice() else {
        panic!("one loop, found {}", loops.len());
    };
    let memory = |b: &BasicBlock| {
        b.instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Load { .. } | Instruction::Store { .. }))
            .count()
    };
    assert_eq!(
        lp.body.iter().map(|&b| memory(&f.blocks[b])).sum::<usize>(),
        0,
        "{f:?}"
    );
    let guard = f.blocks.iter().position(|b| b.label == BlockId(1)).unwrap();
    assert_eq!(
        memory(&f.blocks[guard]),
        0,
        "the guard runs when the loop does not"
    );
    assert_eq!(
        f.blocks.iter().map(memory).sum::<usize>(),
        1 + 1 + 1 + 1,
        "the entry's store, the preheader's load, the exit edge's store, the final load"
    );
    for input in grid() {
        assert_eq!(run(&f, &input), run(&before, &input), "on {input:?}");
    }
}

/// A stand-in for loop-carried forwarding: the load `dest` of the loop's
/// header block is replaced by a phi that takes `init`, loaded in the
/// preheader through `init_ptr`, on the way in and `carried` over the back
/// edge. Only behind a guard, like [`hoist_behind_a_guard`].
fn carry_behind_a_guard(f: &mut IrFunction, dest: u32, init_ptr: u32, carried: u32) -> usize {
    let (cfg, loops) = loops_innermost_first(f);
    let Some(at) = f
        .blocks
        .iter()
        .position(|b| b.instructions.iter().any(|i| i.dest() == Some(Value(dest))))
    else {
        return 0;
    };
    let Some(lp) = loops.iter().find(|lp| lp.body.contains(&at)) else {
        return 0;
    };
    let entries: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p))
        .collect();
    let &[preheader] = entries.as_slice() else {
        return 0;
    };
    if at != lp.header || !matches!(f.blocks[preheader].terminator, Terminator::Branch(_)) {
        return 0;
    }
    let index = f.blocks[at]
        .instructions
        .iter()
        .position(|i| i.dest() == Some(Value(dest)))
        .unwrap();
    f.blocks[at].instructions.remove(index);
    let (from, back) = (f.blocks[preheader].label, f.blocks[at].label);
    let init = 900 + dest;
    f.blocks[preheader].instructions.push(load(init, init_ptr));
    f.blocks[at].instructions.insert(
        0,
        Instruction::Phi {
            dest: Value(dest),
            ty: Ty::I64,
            incoming: vec![(v(init), from), (v(carried), back)],
        },
    );
    1
}

/// `for (i = seed; i < bound; i++) { x = *p; y = x + i; *p = y; }` read as
/// a forwarding problem: the load reads what the trip before stored.
#[test]
fn a_load_carried_in_a_register_pays_whatever_its_address_does() {
    let before = accumulator_behind_a_pointer();
    let mut f = accumulator_behind_a_pointer();
    // The address stays live (the store keeps using it, and the block
    // behind the loop reads through it): a hoisted load would be refused
    // for the register, a carried one is worth it.
    let done = rotate_for_hoisting(&mut f, |f| carry_behind_a_guard(f, 20, 3, 21));
    assert_eq!(done, Hoisted { loops: 1, moved: 1 });
    assert_eq!(violations(&f), Vec::<String>::new());
    for input in grid() {
        assert_eq!(run(&f, &input), run(&before, &input), "on {input:?}");
    }
}

#[test]
fn the_candidate_filter_knows_what_needs_a_guard() {
    let candidate = |f: &IrFunction| {
        let (cfg, loops) = loops_innermost_first(f);
        has_guarded_hoist_candidate(f, &cfg, &loops[0])
    };
    assert!(
        candidate(&invariant_load_in_the_body()),
        "a load of a loop-invariant address"
    );
    assert!(
        !candidate(&counted(IrCmpOp::Slt, false, false, v(10))),
        "arithmetic alone"
    );
    // A load whose address the loop computes moves nowhere.
    let mut variant = invariant_load_in_the_body();
    variant.blocks[2].instructions[0] = load(23, 10);
    assert!(
        !candidate(&variant),
        "a load through the induction variable"
    );
    // A division by a value from outside the loop traps unless guarded.
    let mut division = invariant_load_in_the_body();
    division.blocks[2].instructions[0] = bin(23, IrBinOp::SDiv, v(2), v(3));
    assert!(
        candidate(&division),
        "a division of loop-invariant operands"
    );
    // The same instructions in the header need no guard: LICM takes them
    // from a top-tested loop as it is.
    let mut in_header = invariant_load_in_the_body();
    let moved = in_header.blocks[2].instructions.remove(0);
    in_header.blocks[1].instructions.insert(2, moved);
    assert!(!candidate(&in_header), "a load in the header");
    // `a[i] = a[i-1] + acc`: a load through an address the loop computes
    // next to a store of its type through another. Forwarding the stored
    // value into the next trip's load leaves one load in front of the loop,
    // which needs the guard.
    let recurrence = |stored: Ty, volatile: bool| {
        let mut f = invariant_load_in_the_body();
        f.blocks[2].instructions = vec![
            add(24, v(3), v(10)),
            add(25, v(24), k(-8)),
            load(23, 25),
            add(21, v(20), v(23)),
            Instruction::Store {
                val: v(21),
                ptr: Value(24),
                ty: stored,
                seg_override: AddressSpace::Default,
                volatile,
            },
            add(22, v(10), k(1)),
        ];
        f
    };
    assert!(
        candidate(&recurrence(Ty::I64, false)),
        "a load next to a store of its type"
    );
    assert!(
        !candidate(&recurrence(Ty::I32, false)),
        "a store of another type"
    );
    assert!(!candidate(&recurrence(Ty::I64, true)), "a volatile store");
}
