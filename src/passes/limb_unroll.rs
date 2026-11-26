//! Four limbs a trip: a loop that hands a carry round its back edge is split
//! by its trip count.
//!
//! A limb loop adds two numbers a word at a time:
//!
//! ```c
//! for (i = 0, cy = 0; i < n; i++) {
//!     s = a[i] + b[i];  cy1 = s < a[i];
//!     r[i] = s + cy;    cy = cy1 | (r[i] < s);
//! }
//! ```
//!
//! `passes::carry_chain` makes the body an `adc`. Inside one body the carry
//! lives in the flag; round the loop it cannot, because the loop's own
//! counting and compare write the flags. So every trip takes the carry out
//! (`setc`) at the bottom and puts it back (`bt`) at the top, and that path
//! -- flag, register, flag -- is the recurrence the loop runs at: three
//! steps a limb, where a chain of `adc` takes one.
//!
//! Measured on an Ice Lake-SP, 2,048 limbs 400,000 times: every library
//! loop (GMP, mini-gmp, OpenSSL) runs in 0.75 s, and mini-gmp's `mpn_add_n`
//! written four limbs a trip in the source runs in 0.38 s. This pass writes
//! it that way. A loop `L` of `t` iterations becomes
//!
//! ```text
//!   guard:  is t known exactly, and at least four?
//!   main:   four copies of the body a trip, `t >> 2` trips, nothing
//!           between the copies
//!   L:      the loop as it was, for the `t & 3` iterations that are left,
//!           and for all of them where the guard said no
//! ```
//!
//! and the chain then runs through the four limbs of a trip. The main loop
//! counts its own trips down to zero and steps by constants, and neither
//! needs the carry flag (`dec`, `lea`): the x86 peephole keeps the carry in
//! the flag round its back edge as well
//! (`multiword::keep_carry_round_loop`), and the loop is the chain: 0.26 s.
//!
//! # Which loops
//!
//! Exactly one block of work, tested at the top (`for`, `while`: a header
//! that is phis and the test, and a latch) or at the bottom (`do ...
//! while`: one block). One way in, one way out. A counter stepped by one,
//! up or down, compared with something the loop does not change. The work
//! is loads, stores and arithmetic: nothing that calls, traps into the
//! kernel or is volatile ([`plan_split`] has the list).
//!
//! The header of a loop tested at the top may hold two things besides the
//! compare, both of them the test's own ([`header_work`]): the counter
//! narrowed (`for (int i = 0; i < n; i++) r[i] = ...` arrives with a
//! 64-bit counter, widened to index with, that the test reads through its
//! truncation), and the counter's step (`while (n-- > 0)` steps before it
//! asks).
//!
//! And a carry: a header phi that is the carry in of exactly one full
//! adder in the body and receives that adder's carry out
//! ([`carry_chain::Carried`], the matcher the carry pass itself uses). That
//! is the profit test. Any counted loop could be split this way, and for
//! most it buys only the loop overhead; here it shortens the recurrence.
//! A loop with two adders a trip is left alone (somebody unrolled it).
//!
//! And nothing but the limb in a trip ([`only_a_limb`]): the loads, the
//! stores, the addresses they go through, and the steps of the counter and
//! the pointers. The chain holds from one copy to the next only if nothing
//! between two adders writes the flags. A trip that also sums its limbs
//! (`h += t`) or compares them does, takes the carry out for every limb as
//! before, and is slower in four copies than it was in one (0.78 s as it
//! is, 1.07 s split, for the same 2,048 limbs).
//!
//! # Why it is the same program
//!
//! *The count.* `d` is the distance from the counter's first value to the
//! limit in the direction it steps, computed modulo 2^n in the counter's
//! type (and one more where the test stands behind the work and reads the
//! counter before its step). With a `!=` test the loop runs exactly `d`
//! iterations whatever the values; only `d = 0` is, tested at the bottom,
//! 2^n of them. With `<` or `>` it runs `d` iterations if the first value
//! is on the near side of the limit -- then nothing wraps on the way --
//! and the guard enters only in that case. The guard asks for `d >= 4`,
//! unsigned, which a count of 2^n is not. Everything else goes to `L`
//! untouched, which is the loop the source wrote. An inclusive test (`<=`,
//! `>=`) is taken only against a constant that has a neighbour, as the
//! exclusive test against that.
//!
//! A counter that the test reads narrowed counts in the narrow type. The
//! truncation of a sum is the sum of the truncations, so what the test
//! sees steps by one modulo 2^32 from the truncation of the first value,
//! and `d` is taken from that. The 64-bit counter is carried through the
//! copies as the loop had it, whatever its upper half is.
//!
//! A step in the header is the first thing of an iteration. The loop ran
//! step, test, work, step, test, work, ..., step, test; the copies run step
//! and work four times, in that order, and the step of the test that
//! leaves is `L`'s own, as is every value the header computes for whoever
//! reads it behind the loop: a loop tested at the top is left through
//! `L`'s header and no other way.
//!
//! *The main loop* runs `d >> 2` trips, which it counts down by itself.
//! Each trip is four consecutive iterations of `L` that `L` would have
//! run, in `L`'s order. A store of one iteration is read by the next
//! exactly as it was (the copies keep the order of every load and store),
//! and nothing runs that `L` would not have run: no load is speculated.
//!
//! *The values.* Each copy defines fresh names. What a header phi is in a
//! copy is what the copy before left for it, all phis at once (two phis
//! that swap stay swapped). `L`'s phis receive what the main loop left,
//! and `L` goes on from there: `d & 3` iterations are left, which is what
//! its own test finds. Where the test is at the bottom, `L` must not run
//! when nothing is left, so behind the main loop that is asked, and the
//! values that are read behind the loop are joined in a block of phis
//! between the loop and its exit.
//!
//! *The counter.* Where an iteration reads it for nothing but its step and
//! the test, the main loop does not carry it: behind the main loop it is
//! `d & 3` steps away from where the loop ends, and that is computed.
//!
//! Runs once per function ([`run`] is scheduled in the first round of the
//! pipeline only): the loop it leaves behind has four adders a trip and is
//! not a candidate, and `L` has two ways in.

use super::carry_chain;
use super::loop_analysis;
use super::loop_unroll::rename_inst_dest;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value, replace_values_in_inst_map, replace_values_in_terminator_map,
};

/// Copies of the body in a trip of the main loop.
const COPIES: i64 = 4;

/// The largest body, in instructions, that is copied. The limb loops of
/// GMP, mini-gmp and OpenSSL have between 13 and 19.
const MAX_WORK: usize = 32;

/// A phi of the loop's header: what it is on the way in, and what it
/// receives round the loop.
struct HeaderPhi {
    dest: Value,
    ty: IrType,
    init: Operand,
    back: Operand,
}

/// A loop that will be split, and what the split needs to know.
struct Plan {
    header: usize,
    /// The one block that branches to the header from outside the loop.
    pre: usize,
    /// The test follows the work (`do ... while`), in the header itself.
    bottom_tested: bool,
    /// The block and the instructions that are one iteration.
    work_block: usize,
    work: std::ops::Range<usize>,
    /// Where the loop leaves to.
    exit: BlockId,
    phis: Vec<HeaderPhi>,
    /// Index of the counter among `phis`.
    counter: usize,
    /// A bottom test reads the counter after its step (`--n != 0`), not
    /// before it (`n != 1`).
    tests_next: bool,
    /// The counter is stepped by +1 (else by -1).
    up: bool,
    /// The loop goes on while `counter CONT limit`: `Ne`, or the strict
    /// order in the direction of the step.
    cont: IrCmpOp,
    limit: Operand,
    /// The type the test counts in: the counter's own, or the narrower one
    /// the test reads the counter in.
    count_ty: IrType,
    /// The test reads the counter narrowed (`for (int i = 0; i < n; i++)
    /// r[i] = ...`, whose counter was widened to index with).
    narrowed: bool,
    /// The width of `count_ty`.
    bits: u32,
    /// The counter's step stands in the header, in front of the test
    /// (`while (n-- > 0)`): its place among the header's instructions.
    header_step: Option<usize>,
    /// An iteration reads the counter, or its next value, for something
    /// other than counting (an index, say).
    counter_read: bool,
    /// Values of the loop read behind it, with their types (bottom-tested
    /// loops only: at the top the header's phis are still the last word).
    live_out: Vec<(Value, IrType)>,
}

impl Plan {
    /// One iteration: the counter's step where it stands in the header,
    /// and the work.
    fn iteration(&self, func: &IrFunction) -> Vec<Instruction> {
        iteration_of(
            self.header_step
                .map(|at| &func.blocks[self.header].instructions[at]),
            &func.blocks[self.work_block].instructions[self.work.clone()],
        )
        .into_owned()
    }
}

/// `work` with `step` in front of it, if there is one.
fn iteration_of<'a>(
    step: Option<&'a Instruction>,
    work: &'a [Instruction],
) -> std::borrow::Cow<'a, [Instruction]> {
    match step {
        Some(step) => std::iter::once(step).chain(work).cloned().collect(),
        None => std::borrow::Cow::Borrowed(work),
    }
}

fn counter_bits(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I32 | IrType::U32 => Some(32),
        IrType::I64 | IrType::U64 => Some(64),
        _ => None,
    }
}

/// The counter of `inst`, if `inst` is a counter narrowed for its test: a
/// conversion of a 64-bit value to 32 bits. Returns the value and the type
/// it is narrowed to.
fn narrowed_counter(inst: &Instruction) -> Option<(Value, IrType)> {
    match inst {
        Instruction::Cast {
            src: Operand::Value(src),
            from_ty,
            to_ty,
            ..
        } if counter_bits(*from_ty) == Some(64) && counter_bits(*to_ty) == Some(32) => {
            Some((*src, *to_ty))
        }
        _ => None,
    }
}

/// What the header of a loop tested at the top holds behind its phis, as
/// places among `extra`.
#[derive(Default)]
struct HeaderWork {
    /// The compare the loop branches on (`while (n)` has none).
    test: Option<usize>,
    /// The counter narrowed for the test.
    narrowed: Option<usize>,
    /// The step of a phi (`while (n-- > 0)` steps before it asks).
    step: Option<usize>,
}

/// The header of a loop tested at the top, behind its phis: `extra`, with
/// `cond` what it branches on. `None` if anything stands there but the
/// test, a phi narrowed and the step of a phi: it would run once an
/// iteration, and once more, and is in no copy.
fn header_work(extra: &[Instruction], phis: &[HeaderPhi], cond: Value) -> Option<HeaderWork> {
    let mut found = HeaderWork::default();
    for (at, inst) in extra.iter().enumerate() {
        let is_phi = |v: Value| phis.iter().any(|phi| phi.dest == v);
        let steps_a_phi = |dest: Value| {
            phis.iter().any(|phi| {
                phi.back == Operand::Value(dest)
                    && stepped(inst, phi.dest, phi.ty, phi.dest, 1).is_some()
            })
        };
        let place = match inst {
            Instruction::Cmp { dest, .. } if *dest == cond => &mut found.test,
            Instruction::Cast { .. } if narrowed_counter(inst).is_some_and(|(v, _)| is_phi(v)) => {
                &mut found.narrowed
            }
            Instruction::BinOp { dest, .. } if steps_a_phi(*dest) => &mut found.step,
            _ => return None,
        };
        if place.replace(at).is_some() {
            return None;
        }
    }
    Some(found)
}

/// `value` read as a signed number of `bits` bits.
fn signed(value: i64, bits: u32) -> i64 {
    (value << (64 - bits)) >> (64 - bits)
}

/// The test as a strict one, or `!=`: `(cont, limit)` such that the loop
/// goes on while `counter cont limit` and `cont` is `Ne` or the strict
/// order the step runs towards. An inclusive test is rewritten against the
/// constant next to its limit, where there is one.
fn strict_test(
    cont: IrCmpOp,
    limit: &Operand,
    up: bool,
    ty: IrType,
    bits: u32,
) -> Option<(IrCmpOp, Operand)> {
    use IrCmpOp::{Ne, Sge, Sgt, Sle, Slt, Uge, Ugt, Ule, Ult};
    match (cont, up) {
        (Ne, _) | (Slt | Ult, true) | (Sgt | Ugt, false) => return Some((cont, limit.clone())),
        _ => {}
    }
    let Operand::Const(c) = limit else {
        return None;
    };
    let value = signed(c.to_i64()?, bits);
    let (most, least) = (i64::MAX >> (64 - bits), i64::MIN >> (64 - bits));
    let mask = u64::MAX >> (64 - bits);
    let unsigned = value as u64 & mask;
    let (strict, next) = match (cont, up) {
        (Sle, true) if value != most => (Slt, value + 1),
        (Ule, true) if unsigned != mask => (Ult, signed((unsigned + 1) as i64, bits)),
        (Sge, false) if value != least => (Sgt, value - 1),
        (Uge, false) if unsigned != 0 => (Ugt, signed((unsigned - 1) as i64, bits)),
        _ => return None,
    };
    Some((strict, Operand::Const(IrConst::from_i64(next, ty))))
}

/// The phis at the top of `head`, each with its two ways in: from `pre`
/// and from `latch`.
fn header_phis(head: &BasicBlock, pre: BlockId, latch: BlockId) -> Option<Vec<HeaderPhi>> {
    let mut phis = Vec::new();
    for inst in &head.instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            break;
        };
        let [first, second] = &incoming[..] else {
            return None;
        };
        let (init, back) = if first.1 == pre && second.1 == latch {
            (first, second)
        } else if first.1 == latch && second.1 == pre {
            (second, first)
        } else {
            return None;
        };
        phis.push(HeaderPhi {
            dest: *dest,
            ty: *ty,
            init: init.0.clone(),
            back: back.0.clone(),
        });
    }
    Some(phis)
}

/// What one iteration defines, if it is loads, stores and arithmetic --
/// each run as often and in the order the loop ran them -- with an
/// unsigned comparison among them (a carry is one) and no reader of
/// `header_own`: the compare of a top-tested loop and the counter narrowed
/// for it, which belong to the header and are in no copy.
fn plain_work(work: &[Instruction], header_own: &[Value]) -> Option<FxHashSet<u32>> {
    let mut defined: FxHashSet<u32> = FxHashSet::default();
    let mut compares_unsigned = false;
    for inst in work {
        match inst {
            Instruction::Load {
                volatile: false, ..
            }
            | Instruction::Store {
                volatile: false, ..
            }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::Select { .. } => {}
            Instruction::Cmp { op, .. } => {
                compares_unsigned |= matches!(op, IrCmpOp::Ult | IrCmpOp::Ugt);
            }
            _ => return None,
        }
        let mut reads_header = false;
        inst.for_each_used_value(|v| reads_header |= header_own.contains(&Value(v)));
        if reads_header {
            return None;
        }
        if let Some(dest) = inst.dest() {
            defined.insert(dest.0);
        }
    }
    compares_unsigned.then_some(defined)
}

/// Whether `phi` is stepped by one in `work`: up (`true`) or down.
fn step_by_one(phi: &HeaderPhi, work: &[Instruction]) -> Option<bool> {
    let bits = counter_bits(phi.ty)?;
    let Operand::Value(next) = phi.back else {
        return None;
    };
    let Instruction::BinOp {
        op, lhs, rhs, ty, ..
    } = work.iter().find(|inst| inst.dest() == Some(next))?
    else {
        return None;
    };
    if counter_bits(*ty) != Some(bits) {
        return None;
    }
    let is_phi = |operand: &Operand| matches!(operand, Operand::Value(v) if *v == phi.dest);
    let step = match op {
        IrBinOp::Add if is_phi(lhs) => rhs,
        IrBinOp::Add if is_phi(rhs) => lhs,
        IrBinOp::Sub if is_phi(lhs) => rhs,
        _ => return None,
    };
    let Operand::Const(step) = step else {
        return None;
    };
    let step = signed(step.to_i64()?, bits);
    let step = if *op == IrBinOp::Sub {
        step.checked_neg()?
    } else {
        step
    };
    match step {
        1 => Some(true),
        -1 => Some(false),
        _ => None,
    }
}

/// The values of the loop in block `header` that are read behind it, with
/// their types: phis first, then what the iteration defines, in its order.
/// Each is joined with its copy from the main loop, so its type has to be
/// known.
fn read_behind_loop(
    func: &IrFunction,
    header: usize,
    phis: &[HeaderPhi],
    work: &[Instruction],
    defined: &FxHashSet<u32>,
) -> Option<Vec<(Value, IrType)>> {
    let mut read_outside: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == header {
            continue;
        }
        let mut note = |v: u32| {
            if defined.contains(&v) || phis.iter().any(|phi| phi.dest.0 == v) {
                read_outside.insert(v);
            }
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut note);
        }
        block.terminator.for_each_used_value(&mut note);
    }
    let mut live_out = Vec::new();
    for phi in phis {
        if read_outside.contains(&phi.dest.0) {
            live_out.push((phi.dest, phi.ty));
        }
    }
    for inst in work {
        let Some(dest) = inst.dest() else {
            continue;
        };
        if !read_outside.contains(&dest.0) {
            continue;
        }
        let ty = match inst {
            Instruction::Load { ty, .. }
            | Instruction::BinOp { ty, .. }
            | Instruction::UnaryOp { ty, .. }
            | Instruction::Select { ty, .. } => *ty,
            Instruction::Cast { to_ty, .. } => *to_ty,
            Instruction::GetElementPtr { .. } => IrType::Ptr,
            _ => return None,
        };
        live_out.push((dest, ty));
    }
    Some(live_out)
}

/// The split of the loop `lp`, if it is one this pass takes.
fn plan_split(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
    carried: &mut Option<carry_chain::Carried>,
) -> Option<Plan> {
    let header = lp.header;
    let preds = cfg.preds.row(header);
    let mut inside = preds
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p));
    let latch = inside.next()?;
    if inside.next().is_some() {
        return None;
    }
    let mut outside = preds
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p));
    let pre = outside.next()?;
    if outside.next().is_some() {
        return None;
    }
    let bottom_tested = latch == header;
    if lp.body.len() != if bottom_tested { 1 } else { 2 } {
        return None;
    }
    let head = &func.blocks[header];
    let (header_label, latch_label, pre_label) =
        (head.label, func.blocks[latch].label, func.blocks[pre].label);
    if !matches!(
        func.blocks[pre].terminator,
        Terminator::Branch(_) | Terminator::CondBranch { .. }
    ) {
        return None;
    }

    let phis = header_phis(head, pre_label, latch_label)?;
    let phi_count = phis.len();

    // The test and the two ways out of it.
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label,
        false_label,
    } = &head.terminator
    else {
        return None;
    };
    let stay = if bottom_tested {
        header_label
    } else {
        latch_label
    };
    let (exit, continue_on_true) = if *true_label == stay && *false_label != stay {
        (*false_label, true)
    } else if *false_label == stay && *true_label != stay {
        (*true_label, false)
    } else {
        return None;
    };
    if exit == header_label || exit == latch_label {
        return None;
    }
    let extra = &head.instructions[phi_count..];
    let (work_block, work, top) = if bottom_tested {
        (
            header,
            phi_count..head.instructions.len(),
            HeaderWork::default(),
        )
    } else {
        // The header is its phis and its test (a compare, or the counter
        // itself), with what the test is made of; the latch comes straight
        // back.
        if !matches!(func.blocks[latch].terminator, Terminator::Branch(to) if to == header_label) {
            return None;
        }
        let top = header_work(extra, &phis, *cond)?;
        (latch, 0..func.blocks[latch].instructions.len(), top)
    };
    if work.is_empty() || work.len() > MAX_WORK {
        return None;
    }
    // One iteration: the work, behind the counter's step where that stands
    // in the header. There it runs in front of the test, so once more than
    // the work does -- for the test that leaves, and that one is the
    // header's own, of the loop as it was.
    let iteration = iteration_of(
        top.step.map(|at| &extra[at]),
        &func.blocks[work_block].instructions[work.clone()],
    );
    let work_insts: &[Instruction] = &iteration;
    // The test: a compare in the header, or a value that is itself the
    // condition (`while (n)`: anything but zero goes on).
    let zero = Operand::Const(IrConst::I64(0));
    let branched_on = Operand::Value(*cond);
    let (op, lhs, rhs, test_ty) = match extra.iter().find(|inst| inst.dest() == Some(*cond)) {
        Some(Instruction::Cmp {
            op, lhs, rhs, ty, ..
        }) => (*op, lhs, rhs, Some(*ty)),
        _ => (IrCmpOp::Ne, &branched_on, &zero, None),
    };
    // At the top the compare and the narrowed counter belong to the
    // header, not to an iteration: no copy has them.
    let header_own: Vec<Value> = [top.test, top.narrowed]
        .into_iter()
        .flatten()
        .filter_map(|at| extra[at].dest())
        .collect();

    let defined = plain_work(work_insts, &header_own)?;
    let phi_at = |v: Value| phis.iter().position(|phi| phi.dest == v);
    // Round the loop a phi receives a constant, a value of the iteration,
    // another phi, or something from outside the loop.
    if phis
        .iter()
        .any(|phi| matches!(phi.back, Operand::Value(v) if header_own.contains(&v)))
    {
        return None;
    }
    let invariant = |operand: &Operand| match operand {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            !defined.contains(&v.0) && phi_at(*v).is_none() && !header_own.contains(v)
        }
    };
    // The counter behind the conversion that narrows it for the test, in
    // the header of a loop tested at the top and in the iteration of one
    // tested at the bottom.
    let narrowing = |v: Value| -> Option<(Value, IrType)> {
        let inst = if bottom_tested {
            work_insts.iter().find(|inst| inst.dest() == Some(v))
        } else {
            top.narrowed
                .map(|at| &extra[at])
                .filter(|inst| inst.dest() == Some(v))
        };
        narrowed_counter(inst?)
    };

    // The counter: a phi stepped by one, compared -- itself at the top, its
    // next value at the bottom, either of them as it is or narrowed --
    // with something the loop leaves alone.
    let (counter, tests_next, up, limit, cont, narrow_to) =
        [(lhs, rhs, op), (rhs, lhs, op.swapped())]
            .into_iter()
            .find_map(|(tested, limit, op)| {
                let Operand::Value(tested) = tested else {
                    return None;
                };
                if !invariant(limit) {
                    return None;
                }
                let (tested, narrow_to) = match narrowing(*tested) {
                    Some((counter, to)) => (counter, Some(to)),
                    None => (*tested, None),
                };
                // At the bottom the source tests the stepped counter (`--n
                // != 0`) and the optimizer may have made that the counter
                // itself against the limit one step back (`n != 1`).
                let (counter, tests_next) = phis.iter().enumerate().find_map(|(at, phi)| {
                    if phi.dest == tested {
                        Some((at, false))
                    } else if bottom_tested
                        && matches!(phi.back, Operand::Value(next) if next == tested)
                    {
                        Some((at, true))
                    } else {
                        None
                    }
                })?;
                let up = step_by_one(&phis[counter], work_insts)?;
                Some((counter, tests_next, up, limit, op, narrow_to))
            })?;
    // What the header holds besides its test is the test's own: the
    // counter it reads narrowed, and the counter's step.
    if top.narrowed.is_some() && narrow_to.is_none() {
        return None;
    }
    if let Some(at) = top.step
        && Some(phis[counter].back.clone()) != extra[at].dest().map(Operand::Value)
    {
        return None;
    }
    let ty = narrow_to.unwrap_or(phis[counter].ty);
    let bits = counter_bits(ty)?;
    if test_ty.is_some_and(|test_ty| counter_bits(test_ty) != Some(bits)) {
        return None;
    }
    // A constant limit in the counter's own width (the zero of `while (n)`
    // has none).
    let limit = match limit {
        Operand::Const(c) => Operand::Const(IrConst::from_i64(signed(c.to_i64()?, bits), ty)),
        value => value.clone(),
    };
    let cont = if continue_on_true {
        cont
    } else {
        cont.inverse()
    };
    let (cont, limit) = strict_test(cont, &limit, up, ty, bits)?;

    // Behind a bottom-tested loop anything it computed may be read.
    let live_out = if bottom_tested {
        read_behind_loop(func, header, &phis, work_insts, &defined)?
    } else {
        Vec::new()
    };

    // Whether the counter is anything but a count: read by an instruction
    // that is neither its own step nor the loop's test, handed to another
    // phi, or read behind the loop. The test counts as a reader when it is
    // part of the iteration and its result is itself read by more than the
    // branch.
    let counter_next = match phis[counter].back {
        Operand::Value(next) => next,
        Operand::Const(_) => return None,
    };
    let is_count = |v: Value| v == phis[counter].dest || v == counter_next;
    let handed_on = |wanted: &dyn Fn(Value) -> bool| {
        phis.iter()
            .enumerate()
            .any(|(at, phi)| at != counter && matches!(phi.back, Operand::Value(v) if wanted(v)))
            || live_out.iter().any(|(v, _)| wanted(*v))
    };
    let reads = |inst: &Instruction, wanted: &dyn Fn(Value) -> bool| {
        let mut found = false;
        inst.for_each_used_value(|v| found |= wanted(Value(v)));
        found
    };
    let own_test = (bottom_tested && test_ty.is_some()).then_some(*cond);
    let is_test = |v: Value| own_test == Some(v);
    let test_is_read = work_insts.iter().any(|inst| reads(inst, &is_test)) || handed_on(&is_test);
    // A counter the test reads narrowed is carried as it is: what it is
    // behind the main loop is not a matter of the count alone.
    let counter_read = narrow_to.is_some()
        || handed_on(&is_count)
        || work_insts.iter().any(|inst| {
            let counts_only = inst.dest() == Some(counter_next)
                || (!test_is_read && inst.dest().is_some_and(is_test));
            !counts_only && reads(inst, &is_count)
        });

    // The profit: a carry that goes round the loop through one adder, in
    // an iteration that is nothing but that adder and its addresses.
    let blocks: &[usize] = if bottom_tested {
        &[header]
    } else {
        &[header, latch]
    };
    let carried = carried.get_or_insert_with(|| carry_chain::Carried::of(func));
    let (carry, adder) = phis.iter().enumerate().find_map(|(at, phi)| {
        if at == counter {
            return None;
        }
        let adder = carried.adder_round_loop(func, phi.dest, &phi.back, blocks)?;
        Some((at, adder))
    })?;
    let read_behind = phis
        .iter()
        .enumerate()
        .filter(|&(at, _)| at != carry)
        .filter_map(|(_, phi)| match phi.back {
            Operand::Value(v) => Some(v),
            Operand::Const(_) => None,
        })
        .chain(live_out.iter().map(|(v, _)| *v))
        .chain(own_test);
    if !only_a_limb(work_insts, &adder, read_behind, own_test, &|v| {
        phi_at(v).is_some()
    }) {
        return None;
    }

    Some(Plan {
        header,
        pre,
        bottom_tested,
        work_block,
        work,
        exit,
        phis,
        counter,
        tests_next,
        up,
        cont,
        limit,
        count_ty: ty,
        narrowed: narrow_to.is_some(),
        bits,
        header_step: top.step.map(|at| phi_count + at),
        counter_read,
        live_out,
    })
}

/// Whether one iteration, once its adder is an `adc`, is a limb and nothing
/// else: loads, stores, the addresses they go through, and the steps of
/// the counter and the pointers.
///
/// That is what the split is for. The carry stays in the flag from one
/// copy to the next only if nothing between two adders writes the flags,
/// and moves and addresses do not, while a sum, a compare or a shift of
/// the data does. A trip with such work between its limbs takes the carry
/// out and puts it back for every limb, as the loop did, in four times the
/// code and with four times the values to keep (measured, 2,048 limbs
/// 400,000 times: a loop that also sums its limbs, `h += t`, runs in
/// 0.78 s as it is and in 1.07 s split).
///
/// `work` is the iteration; `adder` its full adder, whose own arithmetic
/// goes when the carry pass rewrites it and is not looked at; `read_behind`
/// the values of the iteration that are read after it (round the loop by a
/// phi other than the carry's, behind the loop, by the test); `own_test`
/// the loop's test where it is part of the iteration; `is_phi` says
/// whether a value is a phi of the header.
fn only_a_limb(
    work: &[Instruction],
    adder: &carry_chain::LoopAdder,
    read_behind: impl Iterator<Item = Value>,
    own_test: Option<Value>,
    is_phi: &dyn Fn(Value) -> bool,
) -> bool {
    let at = |v: Value| work.iter().position(|inst| inst.dest() == Some(v));
    let read_behind: Vec<Value> = read_behind.collect();
    // What is computed when the adder is two instructions: everything a
    // store, the adder's words or a later reader needs, down to the adder
    // itself.
    let mut live = vec![false; work.len()];
    let mut pending = read_behind.clone();
    for operand in &adder.words {
        if let Operand::Value(v) = operand {
            pending.push(*v);
        }
    }
    for (i, inst) in work.iter().enumerate() {
        if matches!(inst, Instruction::Store { .. }) {
            live[i] = true;
            inst.for_each_used_value(|v| pending.push(Value(v)));
        }
    }
    while let Some(v) = pending.pop() {
        if v == adder.word || v == adder.out {
            continue;
        }
        let Some(i) = at(v) else {
            continue;
        };
        if !std::mem::replace(&mut live[i], true) {
            work[i].for_each_used_value(|v| pending.push(Value(v)));
        }
    }
    let constant = |operand: &Operand| matches!(operand, Operand::Const(_));
    let whole = |ty: IrType| ty == IrType::Ptr || counter_bits(ty).is_some() || ty.size() < 4;
    // Whether `value` is part of an address and of nothing else: read by
    // addresses, by loads and stores as where they go, and by extensions
    // and arithmetic of which the same holds -- and by nothing behind the
    // iteration.
    let address_only = |value: Value| {
        let mut pending = vec![value];
        let mut seen = vec![value];
        while let Some(value) = pending.pop() {
            if read_behind.contains(&value) {
                return false;
            }
            for (reader, _) in work.iter().zip(&live).filter(|(_, live)| **live) {
                let mut reads = false;
                reader.for_each_used_value(|v| reads |= v == value.0);
                if !reads {
                    continue;
                }
                match reader {
                    Instruction::GetElementPtr { .. } => {}
                    Instruction::Load { ptr, .. } if *ptr == value => {}
                    Instruction::Store { ptr, val, .. }
                        if *ptr == value && *val != Operand::Value(value) => {}
                    Instruction::Cast { dest, .. } | Instruction::BinOp { dest, .. } => {
                        if !seen.contains(dest) {
                            seen.push(*dest);
                            pending.push(*dest);
                        }
                    }
                    _ => return false,
                }
            }
        }
        true
    };
    work.iter().zip(&live).all(|(inst, &live)| {
        if !live {
            return true;
        }
        match inst {
            Instruction::Load { .. }
            | Instruction::Store { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Copy { .. } => true,
            // An extension or a truncation is a move.
            Instruction::Cast { from_ty, to_ty, .. } => whole(*from_ty) && whole(*to_ty),
            Instruction::Cmp { dest, .. } => own_test == Some(*dest),
            Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } => {
                // The step of a phi by a constant: the counter, a pointer.
                let steps = |phi: &Operand, by: &Operand| {
                    matches!(phi, Operand::Value(v) if is_phi(*v)) && constant(by)
                };
                if match op {
                    IrBinOp::Add => steps(lhs, rhs) || steps(rhs, lhs),
                    IrBinOp::Sub => steps(lhs, rhs),
                    _ => false,
                } {
                    return true;
                }
                // Part of an address, which the access folds into its
                // operand: an index scaled by the size of a word or moved
                // by a constant, read by nothing but the address.
                let folds = match (op, rhs) {
                    (IrBinOp::Add | IrBinOp::Sub, by) => constant(by),
                    (IrBinOp::Shl, Operand::Const(by)) => {
                        by.to_i64().is_some_and(|by| (0..=3).contains(&by))
                    }
                    (IrBinOp::Mul, Operand::Const(by)) => {
                        by.to_i64().is_some_and(|by| matches!(by, 1 | 2 | 4 | 8))
                    }
                    _ => false,
                };
                folds && address_only(*dest)
            }
            _ => false,
        }
    })
}

/// `inst` with its constant step taken `times` over from `from`: for an
/// instruction that steps the value `phi` by a constant (`phi + c`,
/// `phi - c`, the address `c` behind `phi`), the instruction that steps
/// `from` by `times * c`. `None` for anything else.
///
/// The copies of a trip use it to name what a counter or a pointer is in
/// each of them straight from the trip's own phi (`p + 8`, `p + 16`, ...),
/// not from the copy before (`((p + 8) + 8) + 8`): a load then addresses
/// `16(p)` and nothing has to be added between two limbs, where an `add`
/// between them would write the flags the carry is in. Addition wraps, so
/// both spellings are the same value.
///
/// A pointer (`phi_ty`) is stepped as an address, as `address_form` makes
/// one of a pointer's sum that is loaded through: `p + 16` written as the
/// arithmetic it is in the source left the backend to compute some of the
/// copies' addresses into registers of their own (`lea 8(p), r;
/// adc (r), ...`), where the address is folded into the load and the `adc`
/// (`adc 8(p), ...`).
fn stepped(
    inst: &Instruction,
    phi: Value,
    phi_ty: IrType,
    from: Value,
    times: i64,
) -> Option<Instruction> {
    let is_phi = |operand: &Operand| matches!(operand, Operand::Value(v) if *v == phi);
    let scaled = |step: &Operand, negated: bool| -> Option<i64> {
        let Operand::Const(step) = step else {
            return None;
        };
        let step = step.to_i64()?.checked_mul(times)?;
        if negated {
            step.checked_neg()
        } else {
            Some(step)
        }
    };
    match inst {
        Instruction::BinOp {
            dest,
            op: op @ (IrBinOp::Add | IrBinOp::Sub),
            lhs,
            rhs,
            ty,
        } => {
            let step = if is_phi(lhs) {
                rhs
            } else if *op == IrBinOp::Add && is_phi(rhs) {
                lhs
            } else {
                return None;
            };
            if phi_ty == IrType::Ptr && matches!(ty, IrType::Ptr | IrType::I64 | IrType::U64) {
                return Some(Instruction::GetElementPtr {
                    dest: *dest,
                    base: from,
                    offset: Operand::Const(IrConst::I64(scaled(step, *op == IrBinOp::Sub)?)),
                    ty: *ty,
                });
            }
            Some(Instruction::BinOp {
                dest: *dest,
                op: *op,
                lhs: Operand::Value(from),
                rhs: Operand::Const(IrConst::from_i64(scaled(step, false)?, *ty)),
                ty: *ty,
            })
        }
        Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty,
        } if *base == phi => Some(Instruction::GetElementPtr {
            dest: *dest,
            base: from,
            offset: Operand::Const(IrConst::I64(scaled(offset, false)?)),
            ty: *ty,
        }),
        _ => None,
    }
}

fn block(label: BlockId, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label,
        instructions,
        terminator,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

/// Fresh names for the values a split adds.
struct Names {
    next: u32,
}

impl Names {
    fn fresh(&mut self) -> Value {
        self.next += 1;
        Value(self.next - 1)
    }
}

/// The blocks a split adds: the guard, the way into the main loop, the
/// main loop, what follows it, and (bottom-tested loops only) the join in
/// front of the exit.
struct Labels {
    guard: BlockId,
    enter: BlockId,
    main: BlockId,
    after: BlockId,
    join: BlockId,
}

/// The guard: its instructions, the distance and the condition to split.
///
/// The distance is how far the counter is from the limit, the way it
/// steps: the number of iterations (modulo 2^n), where the test reads the
/// counter before the body, or behind the body and the step. Read behind
/// the body and before the step, the test passes once more.
fn guard_of(plan: &Plan, names: &mut Names) -> (Vec<Instruction>, Value, Value) {
    let counter = &plan.phis[plan.counter];
    let ty = plan.count_ty;
    let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
    let mut insts = Vec::new();
    // The counter's first value as the test reads it. A narrowed counter
    // counts in the narrow type: what the test sees of it steps by one
    // there, from this.
    let first = if plan.narrowed {
        let first = names.fresh();
        insts.push(Instruction::Cast {
            dest: first,
            src: counter.init.clone(),
            from_ty: counter.ty,
            to_ty: ty,
        });
        Operand::Value(first)
    } else {
        counter.init.clone()
    };
    let mut distance = names.fresh();
    let (near, far) = if plan.up {
        (first.clone(), plan.limit.clone())
    } else {
        (plan.limit.clone(), first.clone())
    };
    insts.push(Instruction::BinOp {
        dest: distance,
        op: IrBinOp::Sub,
        lhs: far,
        rhs: near,
        ty,
    });
    if plan.bottom_tested && !plan.tests_next {
        let one_more = names.fresh();
        insts.push(Instruction::BinOp {
            dest: one_more,
            op: IrBinOp::Add,
            lhs: Operand::Value(distance),
            rhs: constant(1),
            ty,
        });
        distance = one_more;
    }
    // At least one trip of the main loop. A count of nothing stays with the
    // loop as it was: tested at the top it is no iteration, tested at the
    // bottom it is 2^n of them (`do ... while (--n != 0)` entered with
    // zero).
    let enough = names.fresh();
    insts.push(Instruction::Cmp {
        dest: enough,
        op: IrCmpOp::Uge,
        lhs: Operand::Value(distance),
        rhs: constant(COPIES),
        ty,
    });
    // With an ordered test the distance is the count only from the near
    // side of the limit.
    let split = if plan.cont == IrCmpOp::Ne {
        enough
    } else {
        let near_side = names.fresh();
        insts.push(Instruction::Cmp {
            dest: near_side,
            op: plan.cont,
            lhs: first,
            rhs: plan.limit.clone(),
            ty,
        });
        let both = names.fresh();
        insts.push(Instruction::BinOp {
            dest: both,
            op: IrBinOp::And,
            lhs: Operand::Value(near_side),
            rhs: Operand::Value(enough),
            ty: IrType::I32,
        });
        both
    };
    (insts, distance, split)
}

/// How often each value is read by the blocks `blocks` of `func`.
fn reads_of(func: &IrFunction, blocks: impl Iterator<Item = usize>) -> FxHashMap<u32, u32> {
    let mut reads: FxHashMap<u32, u32> = FxHashMap::default();
    for bi in blocks {
        let block = &func.blocks[bi];
        let mut count = |v: u32| *reads.entry(v).or_default() += 1;
        for inst in &block.instructions {
            inst.for_each_used_value(&mut count);
        }
        block.terminator.for_each_used_value(&mut count);
    }
    reads
}

/// The order a copy is written in: the work's own, without the
/// instructions `late` names, and with an address that one load or store
/// alone goes through directly in front of it.
///
/// `_addcarry_u64(c, a[i], b[i], &r[i])` computes `&r[i]` ahead of the
/// adder and stores through it behind: four copies of that keep one value
/// more alive at every adder, which was one more than the allocator had
/// registers for, and the carry between two copies went through the
/// stack. `is_step` names the instructions that step a phi (they stay
/// where they are) and `read_once` the values with one reader in the
/// function.
fn copy_order<'a>(
    work: &'a [Instruction],
    is_step: &dyn Fn(Value) -> bool,
    read_once: &dyn Fn(Value) -> bool,
    late: &dyn Fn(&Instruction) -> bool,
) -> Vec<&'a Instruction> {
    let mut in_front_of: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    let mut moved: FxHashSet<usize> = FxHashSet::default();
    for (at, inst) in work.iter().enumerate() {
        let Instruction::GetElementPtr { dest, .. } = inst else {
            continue;
        };
        if is_step(*dest) || !read_once(*dest) {
            continue;
        }
        let through = work.iter().position(|user| {
            matches!(
                user,
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } if ptr == dest
            )
        });
        if let Some(through) = through.filter(|through| *through > at + 1) {
            in_front_of.entry(through).or_default().push(at);
            moved.insert(at);
        }
    }
    (0..work.len())
        .filter(|at| !moved.contains(at))
        .flat_map(|at| {
            in_front_of
                .get(&at)
                .into_iter()
                .flatten()
                .copied()
                .chain(std::iter::once(at))
        })
        .map(|at| &work[at])
        .filter(|inst| !late(inst))
        .collect()
}

/// What a phi receives round the loop, as the copies read it: a value of
/// the iteration (or of another phi, or from outside), or a constant
/// under the name it was given in front of the copies.
#[derive(Clone, Copy)]
enum Back {
    Value(Value),
    Named(Value),
}

/// The main loop.
struct MainLoop {
    /// Its phis and the four copies.
    insts: Vec<Instruction>,
    /// The condition it goes round on.
    again: Value,
    /// What each phi is behind it.
    left: Vec<Value>,
    /// What the phis were in the last copy, and the names of what that
    /// copy computed.
    last_phis: Vec<Value>,
    last_names: FxHashMap<u32, u32>,
}

/// Write the main loop: `trips` trips of four copies of the work.
fn main_loop_of(
    func: &IrFunction,
    plan: &Plan,
    names: &mut Names,
    labels: &Labels,
    trips: Value,
) -> MainLoop {
    let ty = plan.count_ty;
    let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
    let work_insts = plan.iteration(func);
    let main_phis: Vec<Value> = plan.phis.iter().map(|_| names.fresh()).collect();
    let mut body = Vec::with_capacity(work_insts.len() * COPIES as usize + plan.phis.len() + 2);
    // A constant a phi receives round the loop is read by the copies under
    // a name.
    let backs: Vec<Back> = plan
        .phis
        .iter()
        .map(|phi| match &phi.back {
            Operand::Const(c) => {
                let named = names.fresh();
                body.push(Instruction::Copy {
                    dest: named,
                    src: Operand::Const(*c),
                });
                Back::Named(named)
            }
            Operand::Value(v) => Back::Value(*v),
        })
        .collect();
    // How often each value of the function is read.
    let reads = reads_of(func, 0..func.blocks.len());
    let read_once = |value: Value| reads.get(&value.0) == Some(&1);
    // And how often by the loop itself: its phis, its iteration, its test.
    let latch = (plan.work_block != plan.header).then_some(plan.work_block);
    let reads_in_loop = reads_of(func, std::iter::once(plan.header).chain(latch));
    // The instruction that steps each phi by a constant, if one does; and
    // whether nothing in the loop but the phi reads what it defines. Such
    // a step is not needed where the source wrote it: `*p++` steps the
    // pointer ahead of the load, and written there the stepped pointer of
    // the last copy is alive while the trip's own pointer is still read,
    // in a register of its own with a move back at the bottom. It is
    // written where the next copy starts, and the last one behind the last
    // copy -- which is also the one that is read behind the loop, if any
    // is (`b - b0` after it).
    let steps: Vec<Option<(&Instruction, bool)>> = plan
        .phis
        .iter()
        .map(|phi| {
            let Operand::Value(next) = phi.back else {
                return None;
            };
            let inst = work_insts.iter().find(|inst| inst.dest() == Some(next))?;
            stepped(inst, phi.dest, phi.ty, phi.dest, 1)?;
            Some((inst, reads_in_loop.get(&next.0) == Some(&1)))
        })
        .collect();
    let step_of: FxHashMap<u32, usize> = steps
        .iter()
        .enumerate()
        .filter_map(|(at, step)| Some((step.as_ref()?.0.dest()?.0, at)))
        .collect();
    let step_from_trip = |at: usize, times: i64, named: Option<Value>| -> Option<Instruction> {
        let (inst, _) = steps[at]?;
        let phi = &plan.phis[at];
        let mut step = stepped(inst, phi.dest, phi.ty, main_phis[at], times)?;
        if let (Some(named), Some(dest)) = (named, inst.dest()) {
            rename_inst_dest(&mut step, &FxHashMap::from_iter([(dest.0, named.0)]));
        }
        Some(step)
    };
    let late = |inst: &Instruction| {
        inst.dest()
            .and_then(|dest| step_of.get(&dest.0))
            .is_some_and(|&at| steps[at].is_some_and(|(_, late)| late))
    };
    let in_order = copy_order(
        &work_insts,
        &|dest| step_of.contains_key(&dest.0),
        &read_once,
        &late,
    );
    // What each phi is in the copy being written; and, of the last copy,
    // what the phis were and the names of what it computed.
    let mut current = main_phis.clone();
    let mut last_phis = Vec::new();
    let mut last_names: FxHashMap<u32, u32> = FxHashMap::default();
    // The late steps the copy just written owes: (phi, how many steps from
    // the trip's phi, the name the next copy reads).
    let mut owed: Vec<(usize, i64, Value)> = Vec::new();
    for copy in 0..COPIES {
        for (at, times, named) in owed.drain(..) {
            body.extend(step_from_trip(at, times, Some(named)));
        }
        let mut copy_names: FxHashMap<u32, u32> = FxHashMap::default();
        for (phi, now) in plan.phis.iter().zip(&current) {
            copy_names.insert(phi.dest.0, now.0);
        }
        for inst in in_order.iter().copied() {
            if let Some(dest) = inst.dest() {
                copy_names.insert(dest.0, names.fresh().0);
            }
        }
        for inst in in_order.iter().copied() {
            // A step of a phi, from the trip's phi; anything else as it is,
            // on this copy's names.
            let step = inst
                .dest()
                .and_then(|dest| step_of.get(&dest.0))
                .and_then(|&at| step_from_trip(at, copy + 1, None));
            let mut inst = match step {
                Some(step) => step,
                None => {
                    let mut inst = inst.clone();
                    replace_values_in_inst_map(&mut inst, &copy_names);
                    inst
                }
            };
            rename_inst_dest(&mut inst, &copy_names);
            body.push(inst);
        }
        // What the copy leaves for each phi: all of them read before any
        // is replaced.
        let left: Vec<Value> = backs
            .iter()
            .enumerate()
            .map(|(at, back)| match *back {
                Back::Named(named) => named,
                Back::Value(_) if steps[at].is_some_and(|(_, late)| late) => {
                    let named = names.fresh();
                    owed.push((at, copy + 1, named));
                    named
                }
                Back::Value(v) => Value(copy_names.get(&v.0).copied().unwrap_or(v.0)),
            })
            .collect();
        if copy + 1 == COPIES {
            last_phis = std::mem::take(&mut current);
            last_names = copy_names;
        }
        current = left;
    }
    for (at, times, named) in owed.drain(..) {
        body.extend(step_from_trip(at, times, Some(named)));
        // Behind the loop the step is read under the name of the last one.
        if let Some(dest) = steps[at].and_then(|(inst, _)| inst.dest()) {
            last_names.insert(dest.0, named.0);
        }
    }
    // The count of trips, last: nothing stands between it and the branch.
    let trip = names.fresh();
    let trips_left = names.fresh();
    body.push(Instruction::BinOp {
        dest: trips_left,
        op: IrBinOp::Sub,
        lhs: Operand::Value(trip),
        rhs: constant(1),
        ty,
    });
    let again = names.fresh();
    body.push(Instruction::Cmp {
        dest: again,
        op: IrCmpOp::Ne,
        lhs: Operand::Value(trips_left),
        rhs: constant(0),
        ty,
    });
    let mut insts: Vec<Instruction> = plan
        .phis
        .iter()
        .zip(&main_phis)
        .zip(&current)
        .map(|((phi, dest), left)| Instruction::Phi {
            dest: *dest,
            ty: phi.ty,
            incoming: vec![
                (phi.init.clone(), labels.enter),
                (Operand::Value(*left), labels.main),
            ],
        })
        .collect();
    insts.push(Instruction::Phi {
        dest: trip,
        ty,
        incoming: vec![
            (Operand::Value(trips), labels.enter),
            (Operand::Value(trips_left), labels.main),
        ],
    });
    insts.append(&mut body);
    MainLoop {
        insts,
        again,
        left: current,
        last_phis,
        last_names,
    }
}

/// The loop's counter behind the main loop, where an iteration does
/// nothing with it but count and the main loop did not carry it: it is
/// `rest` steps away from where the loop ends -- the limit, and one step
/// further where the test reads the counter before it is stepped. The
/// instructions that compute it are added to `insts`.
fn counter_behind(
    plan: &Plan,
    names: &mut Names,
    rest: Value,
    insts: &mut Vec<Instruction>,
) -> Value {
    let ty = plan.phis[plan.counter].ty;
    let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
    let step_back = i64::from(plan.bottom_tested && !plan.tests_next);
    let toward_start = if plan.up { step_back } else { -step_back };
    let base = match &plan.limit {
        Operand::Const(c) => c
            .to_i64()
            .map(|limit| constant(signed(limit.wrapping_add(toward_start), plan.bits))),
        _ => None,
    };
    let base = match base {
        Some(base) => base,
        None if toward_start == 0 => plan.limit.clone(),
        None => {
            let moved = names.fresh();
            insts.push(Instruction::BinOp {
                dest: moved,
                op: IrBinOp::Add,
                lhs: plan.limit.clone(),
                rhs: constant(toward_start),
                ty,
            });
            Operand::Value(moved)
        }
    };
    let at_zero = matches!(&base, Operand::Const(c) if c.to_i64() == Some(0));
    if at_zero && !plan.up {
        return rest;
    }
    let stop = names.fresh();
    insts.push(Instruction::BinOp {
        dest: stop,
        op: if plan.up { IrBinOp::Sub } else { IrBinOp::Add },
        lhs: base,
        rhs: Operand::Value(rest),
        ty,
    });
    stop
}

/// The join in front of the exit of a bottom-tested loop: what is read
/// behind the loop comes from the loop or from the last copy of the main
/// loop. The readers are renamed and the loop leaves through the join.
fn join_behind(
    func: &mut IrFunction,
    plan: &Plan,
    names: &mut Names,
    labels: &Labels,
    main_loop: &MainLoop,
) -> BasicBlock {
    let header_label = func.blocks[plan.header].label;
    let mut joined: FxHashMap<u32, u32> = FxHashMap::default();
    let mut join_insts = Vec::with_capacity(plan.live_out.len());
    for (value, ty) in &plan.live_out {
        let copy = match plan.phis.iter().position(|phi| phi.dest == *value) {
            Some(at) => main_loop.last_phis[at],
            None => Value(main_loop.last_names[&value.0]),
        };
        let dest = names.fresh();
        joined.insert(value.0, dest.0);
        join_insts.push(Instruction::Phi {
            dest,
            ty: *ty,
            incoming: vec![
                (Operand::Value(*value), header_label),
                (Operand::Value(copy), labels.after),
            ],
        });
    }
    retarget(
        &mut func.blocks[plan.header].terminator,
        plan.exit,
        labels.join,
    );
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        if bi == plan.header {
            continue;
        }
        let is_exit = block.label == plan.exit;
        for inst in &mut block.instructions {
            replace_values_in_inst_map(inst, &joined);
            if is_exit && let Instruction::Phi { incoming, .. } = inst {
                for (_, from) in incoming.iter_mut() {
                    if *from == header_label {
                        *from = labels.join;
                    }
                }
            }
        }
        replace_values_in_terminator_map(&mut block.terminator, &joined);
    }
    block(labels.join, join_insts, Terminator::Branch(plan.exit))
}

/// Split the loop of `plan`. Returns the label of the main loop.
fn apply(func: &mut IrFunction, plan: &Plan) -> BlockId {
    let mut names = Names {
        next: func.sound_next_value_id(),
    };
    let first_label = func
        .blocks
        .iter()
        .map(|block| block.label.0 + 1)
        .max()
        .unwrap_or(0)
        .max(func.next_label);
    let [guard, enter, main, after, join] = [0, 1, 2, 3, 4].map(|at| BlockId(first_label + at));
    let labels = Labels {
        guard,
        enter,
        main,
        after,
        join,
    };
    let header_label = func.blocks[plan.header].label;
    let pre_label = func.blocks[plan.pre].label;
    let ty = plan.count_ty;
    let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));

    let (mut guard_insts, distance, split) = guard_of(plan, &mut names);

    // ── The trips ────────────────────────────────────────────────────────
    // Which the main loop counts down to nothing by one. Where the loop's
    // own counter only counts, they are computed in the guard, which then
    // branches straight into the main loop; where an iteration reads the
    // counter, on the way in. The place decides nothing but the layout:
    // the block layout takes an edge into a loop for the hot one, so with
    // the shift on the way in the loop as it was is laid out in front of
    // the main loop, and a loop that steps its pointers then pays for it
    // in the allocator's linear order (its phis and the main loop's start
    // from the same arguments). Measured over the 14 loops of
    // `tests/regression/carry_chain_adders.c` at -O2: 681 instructions and
    // 5 saved registers this way, 686 and 10 with the shift always on the
    // way in, 708 and 14 with it always in the guard.
    let trips = names.fresh();
    let mut enter_insts = Vec::new();
    let count = Instruction::BinOp {
        dest: trips,
        op: IrBinOp::LShr,
        lhs: Operand::Value(distance),
        rhs: constant(i64::from(COPIES.trailing_zeros())),
        ty,
    };
    if plan.counter_read {
        enter_insts.push(count);
    } else {
        guard_insts.push(count);
    }

    let mut main_loop = main_loop_of(func, plan, &mut names, &labels, trips);

    // ── Behind the main loop ─────────────────────────────────────────────
    // What is left of the count: the iterations the loop as it was still
    // has to run. Asked for where the test is at the bottom, and where the
    // counter is made of it.
    let mut after_insts = Vec::new();
    let rest = (plan.bottom_tested || !plan.counter_read).then(|| {
        let rest = names.fresh();
        after_insts.push(Instruction::BinOp {
            dest: rest,
            op: IrBinOp::And,
            lhs: Operand::Value(distance),
            rhs: constant(COPIES - 1),
            ty,
        });
        rest
    });
    if !plan.counter_read
        && let Some(rest) = rest
    {
        main_loop.left[plan.counter] = counter_behind(plan, &mut names, rest, &mut after_insts);
    }
    // Tested at the top the loop asks before it runs. Tested at the bottom
    // it runs at least once, so here is asked whether anything is left;
    // and what is read behind the loop is joined in a block of phis in
    // front of the exit.
    let mut behind = Vec::new();
    let after_terminator = if let (true, Some(rest)) = (plan.bottom_tested, rest) {
        let more = names.fresh();
        after_insts.push(Instruction::Cmp {
            dest: more,
            op: IrCmpOp::Ne,
            lhs: Operand::Value(rest),
            rhs: constant(0),
            ty,
        });
        behind.push(join_behind(func, plan, &mut names, &labels, &main_loop));
        Terminator::CondBranch {
            cond: Operand::Value(more),
            true_label: header_label,
            false_label: join,
        }
    } else {
        Terminator::Branch(header_label)
    };

    // ── The loop as it was ───────────────────────────────────────────────
    // Its phis: from the guard instead of the block in front, and from
    // behind the main loop.
    let phi_count = plan.phis.len();
    for (inst, left) in func.blocks[plan.header].instructions[..phi_count]
        .iter_mut()
        .zip(&main_loop.left)
    {
        if let Instruction::Phi { incoming, .. } = inst {
            for (_, from) in incoming.iter_mut() {
                if *from == pre_label {
                    *from = guard;
                }
            }
            incoming.push((Operand::Value(*left), after));
        }
    }
    retarget(&mut func.blocks[plan.pre].terminator, header_label, guard);

    func.blocks.push(block(
        guard,
        guard_insts,
        Terminator::CondBranch {
            cond: Operand::Value(split),
            true_label: enter,
            false_label: header_label,
        },
    ));
    func.blocks
        .push(block(enter, enter_insts, Terminator::Branch(main)));
    func.blocks.push(block(
        main,
        main_loop.insts,
        Terminator::CondBranch {
            cond: Operand::Value(main_loop.again),
            true_label: main,
            false_label: after,
        },
    ));
    func.blocks
        .push(block(after, after_insts, after_terminator));
    func.blocks.append(&mut behind);
    func.next_label = first_label + if plan.bottom_tested { 5 } else { 4 };
    func.next_value_id = names.next;
    main
}

/// Name `to` wherever the terminator names `from`.
fn retarget(terminator: &mut Terminator, from: BlockId, to: BlockId) {
    match terminator {
        Terminator::Branch(label) if *label == from => *label = to,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            for label in [true_label, false_label] {
                if *label == from {
                    *label = to;
                }
            }
        }
        _ => {}
    }
}

/// Split the limb loops of `func`. Returns how many there were.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut count = 0;
    // The loops that were split and the main loops made of them: neither
    // is looked at again.
    let mut done: FxHashSet<BlockId> = FxHashSet::default();
    loop {
        if func.blocks.len() < 2 {
            return count;
        }
        let cfg = CfgAnalysis::build(func);
        let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ));
        let mut carried = None;
        let Some(split) = loops
            .iter()
            .filter(|lp| lp.body.len() <= 2 && !done.contains(&func.blocks[lp.header].label))
            .find_map(|lp| plan_split(func, &cfg, lp, &mut carried))
        else {
            return count;
        };
        done.insert(func.blocks[split.header].label);
        done.insert(apply(func, &split));
        count += 1;
    }
}

#[cfg(test)]
#[path = "limb_unroll_tests.rs"]
mod tests;
