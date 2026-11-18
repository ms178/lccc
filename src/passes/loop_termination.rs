//! Whether every loop of a function ends.
//!
//! `pure` and `const` license deleting a call whose result is unused and
//! moving one out of a loop, so [`super::ip_purity`] grants them only to a
//! function that hands control back.  Treating every cycle of the flow graph
//! as "may not end" is sound and far too blunt: it withholds the attributes
//! from every function that counts to a bound, and
//!
//! ```c
//! static int run(int n, int (*f)(int)) {
//!     int s = 0;
//!     for (int i = 0; i < n; i++) s += f(i & 255);
//!     return s;
//! }
//! ```
//!
//! called 3000 times with the same arguments was then called 3000 times
//! (tests/regression/costaware_devirt.c; GCC counts the iterations, keeps the
//! function `const` and calls it once).
//!
//! # The proof
//!
//! A loop is shown to end by one exit test in it that
//!
//! - is passed on every iteration: its block dominates every latch;
//! - compares a *counter* with a value the loop does not change; and
//! - is certain to see a counter value that leaves the loop.
//!
//! A counter is a header phi `x` whose value on every back edge is `x + s`
//! for one constant `s`, taken modulo `2^w` (`w` the width of its type), or
//! such a phi plus a constant: the value the latch tests after the increment
//! is one.  In iteration `k` it holds `x0 + k*s`, so over the iterations it
//! runs through exactly the residues congruent to `x0` modulo `g`, where `g`
//! is the largest power of two dividing `s` -- every value when `s` is odd,
//! every fourth when the loop was unrolled four times.
//!
//! The counter values on which the test leaves the loop form an interval in
//! the order of the predicate, signed or unsigned, and either is an arc of
//! the ring of `w`-bit values.  An arc of `g` consecutive values holds a
//! member of every residue class modulo `g`.  So if the exit arc is at least
//! `g` long the counter enters it within `2^w / g` iterations, and the loop
//! is left through this test then, if not earlier through another.
//!
//! How long the arc is follows from the predicate that keeps the loop
//! running, and for the ordered ones from the bound `b`:
//!
//! | loop runs while | it is left on  | length              |
//! |-----------------|----------------|---------------------|
//! | `x != b`        | `b`            | 1                   |
//! | `x == b`        | all but `b`    | `2^w - 1`           |
//! | `x <  b`        | `[b, MAX]`     | `MAX - b + 1`, >= 1 |
//! | `x <= b`        | `(b, MAX]`     | `MAX - b`, >= 0     |
//! | `x >  b`        | `[MIN, b]`     | `b - MIN + 1`, >= 1 |
//! | `x >= b`        | `[MIN, b)`     | `b - MIN`, >= 0     |
//!
//! A counter that steps by an odd amount therefore ends a loop that tests it
//! with `!=`, `<` or `>` against any bound at all, and every other
//! combination needs a constant bound far enough from the end of its range.
//!
//! Nothing here assumes that arithmetic does not overflow, so the answer
//! holds for unsigned counters and under `-fwrapv`.  That is also why
//! `for (i = 0; i <= n; i++)` is declined -- with `n == UINT_MAX` it does not
//! end, and for an unsigned counter that is defined behaviour -- and why a
//! counter stepping by two against a variable bound is: it can step over the
//! last value below the bound and wrap.  GCC proves those for signed counters
//! and pointers from the absence of overflow; here such a function stays
//! "may not return", which costs it the two transformations above and keeps
//! its calls mergeable ([`crate::ir::reexports::MemoryEffect`]).

use super::loop_analysis::{self, NaturalLoop};
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::analysis::{self, DominanceChecker, FlatAdj};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrFunction, Operand, Terminator, Value,
};

/// How many `+ constant` steps may separate a tested value from its phi.
/// One is the increment a rotated loop tests; unrolling and the affine exit
/// folds add at most one more.
const MAX_OFFSET_CHAIN: usize = 4;

/// The block and the position in it of the instruction defining each value.
type Definitions = FxHashMap<u32, (usize, usize)>;

/// Whether every cycle among the blocks `reachable` from the entry ends.
///
/// `preds`/`succs` are the function's flow graph ([`analysis::build_cfg`],
/// asm-goto edges included).
pub(super) fn every_cycle_ends(
    func: &IrFunction,
    preds: &FlatAdj,
    succs: &FlatAdj,
    reachable: &[bool],
) -> bool {
    let blocks = func.blocks.len();
    // An asm goto leaves its block ahead of the terminator, so the exit test
    // of that block is not passed on every way through it.
    if func
        .blocks
        .iter()
        .any(|block| analysis::asm_goto_successors(block).next().is_some())
    {
        return false;
    }
    // The proof reads an instruction as *the* definition of its value.
    let Some(definitions) = single_definitions(func) else {
        return false;
    };
    let idom = analysis::compute_dominators(blocks, preds, succs);
    let dominance = DominanceChecker::new(blocks, &idom);
    // Every cycle has to be a natural loop for the loops below to be all the
    // cycles there are.
    if !reducible(succs, &dominance, reachable) {
        return false;
    }
    let labels = analysis::build_label_map(func);
    let context = Context {
        func,
        preds,
        dominance: &dominance,
        definitions: &definitions,
        labels: &labels,
        reachable,
    };
    loop_analysis::find_merged_natural_loops(blocks, preds, succs, &idom)
        .iter()
        .filter(|natural| reachable[natural.header])
        .all(|natural| context.ends(natural))
}

/// Where each value is defined, or `None` if one is defined twice.
fn single_definitions(func: &IrFunction) -> Option<Definitions> {
    let mut definitions = Definitions::default();
    for (block, body) in func.blocks.iter().enumerate() {
        for (position, inst) in body.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest()
                && definitions.insert(dest.0, (block, position)).is_some()
            {
                return None;
            }
        }
    }
    Some(definitions)
}

/// Whether every cycle of the reachable graph runs through a back edge, an
/// edge to a block that dominates its source: without those edges the graph
/// must sort topologically.
fn reducible(succs: &FlatAdj, dominance: &DominanceChecker, reachable: &[bool]) -> bool {
    let blocks = reachable.len();
    let forward = |from: usize| {
        succs
            .row(from)
            .iter()
            .map(|&to| to as usize)
            .filter(move |&to| !dominance.dominates(to, from))
    };
    let mut pending = vec![0u32; blocks];
    let mut live = 0usize;
    for from in (0..blocks).filter(|&block| reachable[block]) {
        live += 1;
        for to in forward(from) {
            pending[to] += 1;
        }
    }
    let mut ready: Vec<usize> = (0..blocks)
        .filter(|&block| reachable[block] && pending[block] == 0)
        .collect();
    let mut sorted = 0usize;
    while let Some(from) = ready.pop() {
        sorted += 1;
        for to in forward(from) {
            pending[to] -= 1;
            if pending[to] == 0 {
                ready.push(to);
            }
        }
    }
    sorted == live
}

struct Context<'a> {
    func: &'a IrFunction,
    preds: &'a FlatAdj,
    dominance: &'a DominanceChecker,
    definitions: &'a Definitions,
    labels: &'a FxHashMap<BlockId, usize>,
    reachable: &'a [bool],
}

impl Context<'_> {
    fn definition(&self, value: Value) -> Option<(usize, &Instruction)> {
        let &(block, position) = self.definitions.get(&value.0)?;
        Some((block, &self.func.blocks[block].instructions[position]))
    }

    /// Whether some exit test of `natural` is certain to leave it.
    fn ends(&self, natural: &NaturalLoop) -> bool {
        let latches: Vec<usize> = natural
            .latches(self.preds)
            .into_iter()
            .filter(|&latch| self.reachable[latch])
            .collect();
        natural
            .body
            .iter()
            .any(|&block| self.reachable[block] && self.test_ends(natural, &latches, block))
    }

    /// Whether the conditional branch ending `block` is such a test.
    fn test_ends(&self, natural: &NaturalLoop, latches: &[usize], block: usize) -> bool {
        let Terminator::CondBranch {
            cond: Operand::Value(cond),
            true_label,
            false_label,
        } = &self.func.blocks[block].terminator
        else {
            return false;
        };
        let (Some(&on_true), Some(&on_false)) =
            (self.labels.get(true_label), self.labels.get(false_label))
        else {
            return false;
        };
        let stays_on_true = natural.contains(on_true);
        if stays_on_true == natural.contains(on_false) {
            return false;
        }
        if !latches
            .iter()
            .all(|&latch| self.dominance.dominates(block, latch))
        {
            return false;
        }
        let Some((
            _,
            &Instruction::Cmp {
                op, lhs, rhs, ty, ..
            },
        )) = self.definition(*cond)
        else {
            return false;
        };
        let Some(ring) = Ring::of(ty) else {
            return false;
        };
        // The predicate that keeps the loop running, with the counter on
        // its left.
        let runs_while = if stays_on_true { op } else { op.inverse() };
        [(lhs, rhs, runs_while), (rhs, lhs, runs_while.swapped())]
            .into_iter()
            .any(|(counter, bound, runs_while)| {
                let Operand::Value(counter) = counter else {
                    return false;
                };
                let bound = match bound {
                    Operand::Const(constant) => match constant.to_i64() {
                        Some(constant) => Some(constant),
                        None => return false,
                    },
                    Operand::Value(value) => {
                        if !self.unchanged_in(natural, value) {
                            return false;
                        }
                        None
                    }
                };
                self.step_of(natural, latches, counter, ty)
                    .is_some_and(|step| ring.exit_is_reached(runs_while, step, bound))
            })
    }

    /// Whether `value` is defined outside `natural`: one value for all of its
    /// iterations.  A value with no definition at all is not taken on trust.
    fn unchanged_in(&self, natural: &NaturalLoop, value: Value) -> bool {
        self.definition(value)
            .is_some_and(|(block, _)| !natural.contains(block))
    }

    /// The step of the counter `value` is, if it is one: a header phi of
    /// `natural` that every back edge advances by the same constant, plus
    /// constants.
    fn step_of(
        &self,
        natural: &NaturalLoop,
        latches: &[usize],
        mut value: Value,
        ty: IrType,
    ) -> Option<u64> {
        for _ in 0..=MAX_OFFSET_CHAIN {
            let (block, inst) = self.definition(value)?;
            if let Instruction::Phi {
                dest,
                ty: phi_ty,
                incoming,
            } = inst
            {
                if block != natural.header || *phi_ty != ty {
                    return None;
                }
                return self.phi_step(latches, *dest, incoming, ty);
            }
            value = plus_constant(inst, ty)?.0;
        }
        None
    }

    /// The constant every back edge adds to the header phi `phi`.
    fn phi_step(
        &self,
        latches: &[usize],
        phi: Value,
        incoming: &[(Operand, BlockId)],
        ty: IrType,
    ) -> Option<u64> {
        let mut advanced: Option<Value> = None;
        for &latch in latches {
            let label = self.func.blocks[latch].label;
            let mut named = false;
            for (operand, _) in incoming.iter().filter(|(_, from)| *from == label) {
                named = true;
                let Operand::Value(value) = *operand else {
                    return None;
                };
                if *advanced.get_or_insert(value) != value {
                    return None;
                }
            }
            if !named {
                return None;
            }
        }
        let (_, inst) = self.definition(advanced?)?;
        let (base, step) = plus_constant(inst, ty)?;
        (base == phi && Ring::of(ty)?.reduce(step) != 0).then_some(step)
    }
}

/// `value + constant` or `value - constant` in type `ty`, as the value and
/// what is added to it (modulo the width of `ty`, see [`Ring::reduce`]).
fn plus_constant(inst: &Instruction, ty: IrType) -> Option<(Value, u64)> {
    let &Instruction::BinOp {
        op,
        lhs,
        rhs,
        ty: op_ty,
        ..
    } = inst
    else {
        return None;
    };
    if op_ty != ty {
        return None;
    }
    match (op, lhs, rhs) {
        (IrBinOp::Add, Operand::Value(value), Operand::Const(constant))
        | (IrBinOp::Add, Operand::Const(constant), Operand::Value(value)) => {
            Some((value, constant.to_i64()? as u64))
        }
        (IrBinOp::Sub, Operand::Value(value), Operand::Const(constant)) => {
            Some((value, (constant.to_i64()? as u64).wrapping_neg()))
        }
        _ => None,
    }
}

/// The values of a 32- or 64-bit integer type.
#[derive(Clone, Copy)]
struct Ring {
    bits: u32,
    /// `U32`: its constants fold zero-extended and its signed predicates
    /// with them, which is not the order the instruction selector compares
    /// in.  No such comparison is used in a proof.
    unsigned_only: bool,
}

impl Ring {
    /// Narrower counters are promoted before they are compared and converted
    /// back afterwards; pointers may wrap under `-fwrapv-pointer` like any
    /// other counter, and step by more than one.
    fn of(ty: IrType) -> Option<Self> {
        match ty {
            IrType::I32 => Some(Self {
                bits: 32,
                unsigned_only: false,
            }),
            IrType::U32 => Some(Self {
                bits: 32,
                unsigned_only: true,
            }),
            IrType::I64 | IrType::U64 => Some(Self {
                bits: 64,
                unsigned_only: false,
            }),
            _ => None,
        }
    }

    fn size(self) -> u128 {
        1u128 << self.bits
    }

    /// `value` modulo the size of the ring.
    fn reduce(self, value: u64) -> u64 {
        if self.bits == 64 {
            value
        } else {
            value & ((1u64 << self.bits) - 1)
        }
    }

    /// Whether a counter stepping by `step` (not a multiple of the ring's
    /// size) reaches a value on which a loop that runs while
    /// `counter runs_while bound` is left; `bound` is `None` for a bound
    /// known only not to change.
    fn exit_is_reached(self, runs_while: IrCmpOp, step: u64, bound: Option<i64>) -> bool {
        let step = self.reduce(step);
        debug_assert_ne!(step, 0, "a counter that does not move");
        // The counter visits every `stride`-th value of the ring.
        let stride = 1u128 << step.trailing_zeros();
        let signed = match runs_while {
            IrCmpOp::Eq | IrCmpOp::Ne => false,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => true,
            IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => false,
        };
        if signed && self.unsigned_only {
            return false;
        }
        // How far the bound is from each end of the predicate's order.
        let (below, above) = match bound {
            None => (None, None),
            Some(bound) => {
                let bound = self.reduce(bound as u64);
                let position = if signed {
                    // Offset binary: the signed order from the bottom.
                    u128::from(bound ^ (1u64 << (self.bits - 1)))
                } else {
                    u128::from(bound)
                };
                (Some(position), Some(self.size() - 1 - position))
            }
        };
        let exit_values = match runs_while {
            IrCmpOp::Ne => 1,
            IrCmpOp::Eq => self.size() - 1,
            // Left on `[bound, MAX]`: never empty, whatever the bound.
            IrCmpOp::Slt | IrCmpOp::Ult => above.map_or(1, |above| above + 1),
            // Left on `(bound, MAX]`: empty for the largest bound.
            IrCmpOp::Sle | IrCmpOp::Ule => above.unwrap_or(0),
            // Left on `[MIN, bound]`.
            IrCmpOp::Sgt | IrCmpOp::Ugt => below.map_or(1, |below| below + 1),
            // Left on `[MIN, bound)`.
            IrCmpOp::Sge | IrCmpOp::Uge => below.unwrap_or(0),
        };
        exit_values >= stride
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, IrConst};

    const N: u32 = 100;
    const X: u32 = 1;
    const NEXT: u32 = 2;
    const TEST: u32 = 3;

    fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions,
            terminator,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    fn value(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    fn constant(number: i64, ty: IrType) -> Operand {
        Operand::Const(IrConst::from_i64(number, ty))
    }

    fn param(dest: u32, ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: 0,
            ty,
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

    fn add(dest: u32, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        }
    }

    fn compare(dest: u32, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty,
        }
    }

    fn when(cond: u32, on_true: u32, on_false: u32) -> Terminator {
        Terminator::CondBranch {
            cond: value(cond),
            true_label: BlockId(on_true),
            false_label: BlockId(on_false),
        }
    }

    fn jump(target: u32) -> Terminator {
        Terminator::Branch(BlockId(target))
    }

    fn done() -> Terminator {
        Terminator::Return(None)
    }

    fn function(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut func = IrFunction::new("f".into(), IrType::Void, vec![], false);
        func.blocks = blocks;
        func
    }

    /// The question as [`super::super::ip_purity`] asks it.
    fn ends(blocks: Vec<BasicBlock>) -> bool {
        super::super::ip_purity::body_always_returns(&function(blocks))
    }

    /// `for (x = 0; x OP bound; x += step) body;` with the test in the
    /// header: 0 entry, 1 header, 2 body and latch, 3 exit.
    fn counted(ty: IrType, op: IrCmpOp, bound: Operand, step: i64) -> Vec<BasicBlock> {
        vec![
            block(0, vec![param(N, ty)], jump(1)),
            block(
                1,
                vec![
                    phi(X, ty, vec![(constant(0, ty), 0), (value(NEXT), 2)]),
                    compare(TEST, op, value(X), bound, ty),
                ],
                when(TEST, 2, 3),
            ),
            block(
                2,
                vec![add(NEXT, value(X), constant(step, ty), ty)],
                jump(1),
            ),
            block(3, vec![], done()),
        ]
    }

    const I32: IrType = IrType::I32;
    const U32: IrType = IrType::U32;

    #[test]
    fn a_unit_step_against_any_bound_ends_for_strict_and_unequal_tests() {
        for ty in [I32, IrType::I64, IrType::U64] {
            assert!(ends(counted(ty, IrCmpOp::Slt, value(N), 1)), "{ty:?} <");
            assert!(ends(counted(ty, IrCmpOp::Sgt, value(N), -1)), "{ty:?} >");
        }
        for ty in [I32, U32, IrType::I64, IrType::U64] {
            assert!(ends(counted(ty, IrCmpOp::Ult, value(N), 1)), "{ty:?} <u");
            assert!(ends(counted(ty, IrCmpOp::Ugt, value(N), -1)), "{ty:?} >u");
            assert!(ends(counted(ty, IrCmpOp::Ne, value(N), 1)), "{ty:?} !=");
            assert!(
                ends(counted(ty, IrCmpOp::Ne, value(N), -1)),
                "{ty:?} != down"
            );
        }
        // The direction does not matter: counting away from the bound wraps
        // into the exit arc, `for (size_t i = n - 1; i < n; i--)`.
        assert!(ends(counted(IrType::U64, IrCmpOp::Ult, value(N), -1)));
        // Any odd step visits every value.
        assert!(ends(counted(U32, IrCmpOp::Ne, value(N), 3)));
        assert!(ends(counted(I32, IrCmpOp::Slt, value(N), 7)));
    }

    /// `for (i = 0; i <= n; i++)` does not end for `n == MAX`, and for an
    /// unsigned counter that is defined behaviour.
    #[test]
    fn an_inclusive_test_needs_a_bound_short_of_the_end_of_its_range() {
        assert!(!ends(counted(U32, IrCmpOp::Ule, value(N), 1)));
        assert!(!ends(counted(I32, IrCmpOp::Sle, value(N), 1)));
        assert!(!ends(counted(I32, IrCmpOp::Sge, value(N), -1)));
        assert!(ends(counted(U32, IrCmpOp::Ule, constant(100, U32), 1)));
        assert!(!ends(counted(U32, IrCmpOp::Ule, constant(-1, U32), 1)));
        assert!(ends(counted(I32, IrCmpOp::Sle, constant(100, I32), 1)));
        assert!(!ends(counted(
            I32,
            IrCmpOp::Sle,
            constant(i64::from(i32::MAX), I32),
            1
        )));
        // `for (i = n; i >= 0; i--)`: ends for a signed counter, never for an
        // unsigned one.
        assert!(ends(counted(I32, IrCmpOp::Sge, constant(0, I32), -1)));
        assert!(!ends(counted(U32, IrCmpOp::Uge, constant(0, U32), -1)));
        assert!(!ends(counted(
            I32,
            IrCmpOp::Sge,
            constant(i64::from(i32::MIN), I32),
            -1
        )));
    }

    /// A counter that steps by `2^k` sees every `2^k`-th value, so it must
    /// have `2^k` consecutive exit values to land on one.
    #[test]
    fn a_wider_step_needs_as_many_exit_values_as_it_skips() {
        // Against a variable bound it can step over the last one and wrap.
        assert!(!ends(counted(U32, IrCmpOp::Ult, value(N), 2)));
        assert!(!ends(counted(I32, IrCmpOp::Slt, value(N), 4)));
        assert!(!ends(counted(U32, IrCmpOp::Ne, value(N), 2)));
        assert!(!ends(counted(U32, IrCmpOp::Ne, constant(7, U32), 2)));
        // `for (i = 0; i < 1000; i += 4)`.
        assert!(ends(counted(I32, IrCmpOp::Slt, constant(1000, I32), 4)));
        assert!(ends(counted(U32, IrCmpOp::Ult, constant(1000, U32), 4)));
        // Exactly as many exit values as the stride, and one fewer.
        let top = i64::from(u32::MAX);
        assert!(ends(counted(U32, IrCmpOp::Ult, constant(top - 3, U32), 4)));
        assert!(!ends(counted(U32, IrCmpOp::Ult, constant(top - 2, U32), 4)));
        assert!(ends(counted(U32, IrCmpOp::Ule, constant(top - 4, U32), 4)));
        assert!(!ends(counted(U32, IrCmpOp::Ule, constant(top - 3, U32), 4)));
        let high = i64::from(i32::MAX);
        assert!(ends(counted(I32, IrCmpOp::Slt, constant(high - 3, I32), 4)));
        assert!(!ends(counted(
            I32,
            IrCmpOp::Slt,
            constant(high - 2, I32),
            4
        )));
        // Downwards the same, from the other end; 12 skips four at a time.
        assert!(ends(counted(U32, IrCmpOp::Ugt, constant(3, U32), -4)));
        assert!(!ends(counted(U32, IrCmpOp::Ugt, constant(2, U32), -12)));
        let low = i64::from(i32::MIN);
        assert!(ends(counted(I32, IrCmpOp::Sge, constant(low + 4, I32), -4)));
        assert!(!ends(counted(
            I32,
            IrCmpOp::Sge,
            constant(low + 3, I32),
            -4
        )));
        // 64 bits: the top of the unsigned range is -1.
        let wide = IrType::U64;
        assert!(ends(counted(wide, IrCmpOp::Ult, constant(-8, wide), 8)));
        assert!(!ends(counted(wide, IrCmpOp::Ult, constant(-7, wide), 8)));
    }

    /// `while (x == b) x += s;` is left as soon as the counter moves.
    #[test]
    fn a_loop_that_runs_while_equal_ends() {
        assert!(ends(counted(U32, IrCmpOp::Eq, value(N), 2)));
        assert!(ends(counted(I32, IrCmpOp::Eq, constant(0, I32), -6)));
    }

    #[test]
    fn a_counter_that_does_not_move_proves_nothing() {
        assert!(!ends(counted(U32, IrCmpOp::Ult, value(N), 0)));
        assert!(!ends(counted(U32, IrCmpOp::Eq, value(N), 0)));
        // A multiple of 2^32 is no step of a 32-bit counter.
        let mut blocks = counted(U32, IrCmpOp::Ult, value(N), 1);
        blocks[2].instructions[0] = add(
            NEXT,
            value(X),
            Operand::Const(IrConst::I64(1i64 << 32)),
            U32,
        );
        assert!(!ends(blocks));
    }

    /// The same loop with the arms of its branch exchanged and the
    /// predicate inverted, and with the operands exchanged.
    #[test]
    fn the_branch_and_the_operands_may_be_either_way_round() {
        let mut inverted = counted(I32, IrCmpOp::Sge, value(N), 1);
        inverted[1].terminator = when(TEST, 3, 2);
        assert!(ends(inverted));
        // `x >= n` kept as the condition to stay does not end.
        assert!(!ends(counted(I32, IrCmpOp::Sge, value(N), 1)));

        let mut swapped = counted(I32, IrCmpOp::Sgt, value(N), 1);
        swapped[1].instructions[1] = compare(TEST, IrCmpOp::Sgt, value(N), value(X), I32);
        assert!(ends(swapped));
        let mut swapped = counted(I32, IrCmpOp::Sge, value(N), 1);
        swapped[1].instructions[1] = compare(TEST, IrCmpOp::Sge, value(N), value(X), I32);
        assert!(!ends(swapped), "n >= x is x <= n");
    }

    /// A rotated loop tests the incremented value in its latch, and an
    /// unrolled one a value some constant ahead of that.
    #[test]
    fn the_test_may_read_the_counter_plus_a_constant() {
        let rotated = |tested: u32, extra: Vec<Instruction>| {
            let mut body = vec![
                phi(X, I32, vec![(constant(0, I32), 0), (value(NEXT), 1)]),
                add(NEXT, value(X), constant(1, I32), I32),
            ];
            body.extend(extra);
            body.push(compare(TEST, IrCmpOp::Slt, value(tested), value(N), I32));
            vec![
                block(0, vec![param(N, I32)], jump(1)),
                block(1, body, when(TEST, 1, 2)),
                block(2, vec![], done()),
            ]
        };
        assert!(ends(rotated(NEXT, vec![])));
        assert!(ends(rotated(X, vec![])));
        let ahead = |from: u32, dest: u32| add(dest, constant(3, I32), value(from), I32);
        assert!(ends(rotated(10, vec![ahead(NEXT, 10)])));
        // Subtraction of a constant is addition of its negative ...
        let behind = Instruction::BinOp {
            dest: Value(10),
            op: IrBinOp::Sub,
            lhs: value(NEXT),
            rhs: constant(3, I32),
            ty: I32,
        };
        assert!(ends(rotated(10, vec![behind])));
        // ... but `3 - x` runs the other way and `x * 3` is no offset.
        let mirrored = Instruction::BinOp {
            dest: Value(10),
            op: IrBinOp::Sub,
            lhs: constant(3, I32),
            rhs: value(NEXT),
            ty: I32,
        };
        assert!(!ends(rotated(10, vec![mirrored])));
        let scaled = Instruction::BinOp {
            dest: Value(10),
            op: IrBinOp::Mul,
            lhs: value(NEXT),
            rhs: constant(3, I32),
            ty: I32,
        };
        assert!(!ends(rotated(10, vec![scaled])));
        // The chain of offsets is followed a fixed distance and no further.
        let chain = |length: u32| {
            (0..length)
                .map(|link| ahead(if link == 0 { NEXT } else { 9 + link }, 10 + link))
                .collect::<Vec<_>>()
        };
        let longest = MAX_OFFSET_CHAIN as u32 - 1;
        assert!(ends(rotated(9 + longest, chain(longest))));
        assert!(!ends(rotated(10 + longest, chain(longest + 1))));
    }

    /// A subtraction as the increment: `x = x - 1`.
    #[test]
    fn the_increment_may_be_a_subtraction() {
        let mut down = counted(I32, IrCmpOp::Sgt, constant(0, I32), 1);
        down[2].instructions[0] = Instruction::BinOp {
            dest: Value(NEXT),
            op: IrBinOp::Sub,
            lhs: value(X),
            rhs: constant(1, I32),
            ty: I32,
        };
        assert!(ends(down));
        // `x = 1 - x` alternates between two values.
        let mut flips = counted(I32, IrCmpOp::Ne, value(N), 1);
        flips[2].instructions[0] = Instruction::BinOp {
            dest: Value(NEXT),
            op: IrBinOp::Sub,
            lhs: constant(1, I32),
            rhs: value(X),
            ty: I32,
        };
        assert!(!ends(flips));
    }

    #[test]
    fn a_bound_the_loop_changes_proves_nothing() {
        // The bound is loaded anew in the header: `while (i < *limit)`.
        let mut reloaded = counted(I32, IrCmpOp::Slt, value(20), 1);
        reloaded[1]
            .instructions
            .insert(1, add(20, value(N), constant(0, I32), I32));
        assert!(!ends(reloaded));
        // The same definition ahead of the loop is one value throughout.
        let mut hoisted = counted(I32, IrCmpOp::Slt, value(20), 1);
        hoisted[0]
            .instructions
            .push(add(20, value(N), constant(0, I32), I32));
        assert!(ends(hoisted));
        // A value nothing defines is not taken on trust.
        assert!(!ends(counted(I32, IrCmpOp::Slt, value(77), 1)));
        // Two counters racing: neither is a bound for the other.
        let mut racing = counted(I32, IrCmpOp::Slt, value(20), 1);
        racing[1]
            .instructions
            .insert(1, phi(20, I32, vec![(constant(9, I32), 0), (value(21), 2)]));
        racing[2]
            .instructions
            .push(add(21, value(20), constant(1, I32), I32));
        assert!(!ends(racing));
    }

    /// `for (i = 0; ; i++) { if (c) { if (i >= n) break; } }`
    #[test]
    fn a_test_that_an_iteration_can_avoid_proves_nothing() {
        let skipped = vec![
            block(0, vec![param(N, I32), param(50, I32)], jump(1)),
            block(
                1,
                vec![
                    phi(X, I32, vec![(constant(0, I32), 0), (value(NEXT), 3)]),
                    add(NEXT, value(X), constant(1, I32), I32),
                    compare(TEST, IrCmpOp::Slt, value(X), value(N), I32),
                ],
                when(50, 2, 3),
            ),
            block(2, vec![], when(TEST, 3, 4)),
            block(3, vec![], jump(1)),
            block(4, vec![], done()),
        ];
        assert!(!ends(skipped));
    }

    /// `continue` gives the loop a second latch; both must advance the
    /// counter, and by the same amount.
    #[test]
    fn every_back_edge_must_advance_the_counter_alike() {
        let two_latches = |other: Operand, extra: Vec<Instruction>| {
            let mut body = vec![add(NEXT, value(X), constant(1, I32), I32)];
            body.extend(extra);
            vec![
                block(0, vec![param(N, I32), param(50, I32)], jump(1)),
                block(
                    1,
                    vec![
                        phi(
                            X,
                            I32,
                            vec![(constant(0, I32), 0), (value(NEXT), 2), (other, 3)],
                        ),
                        compare(TEST, IrCmpOp::Slt, value(X), value(N), I32),
                    ],
                    when(TEST, 2, 4),
                ),
                block(2, body, when(50, 1, 3)),
                block(3, vec![], jump(1)),
                block(4, vec![], done()),
            ]
        };
        assert!(ends(two_latches(value(NEXT), vec![])));
        // `continue` without the increment.
        assert!(!ends(two_latches(value(X), vec![])));
        // A second path that steps by two.
        let twice = add(30, value(X), constant(2, I32), I32);
        assert!(!ends(two_latches(value(30), vec![twice])));
        assert!(!ends(two_latches(constant(0, I32), vec![])));
        // A back edge the phi does not name at all.
        let mut unnamed = two_latches(value(NEXT), vec![]);
        if let Instruction::Phi { incoming, .. } = &mut unnamed[1].instructions[0] {
            incoming.pop();
        }
        assert!(!ends(unnamed));
    }

    #[test]
    fn every_loop_of_a_nest_needs_its_own_proof() {
        // for (x = 0; x < n; x++) for (y = 0; y < x; y++) ;
        let nest = |inner: IrCmpOp| {
            vec![
                block(0, vec![param(N, I32)], jump(1)),
                block(
                    1,
                    vec![
                        phi(X, I32, vec![(constant(0, I32), 0), (value(NEXT), 4)]),
                        compare(TEST, IrCmpOp::Slt, value(X), value(N), I32),
                    ],
                    when(TEST, 2, 5),
                ),
                block(
                    2,
                    vec![
                        phi(11, I32, vec![(constant(0, I32), 1), (value(12), 3)]),
                        compare(13, inner, value(11), value(X), I32),
                    ],
                    when(13, 3, 4),
                ),
                block(3, vec![add(12, value(11), constant(1, I32), I32)], jump(2)),
                block(4, vec![add(NEXT, value(X), constant(1, I32), I32)], jump(1)),
                block(5, vec![], done()),
            ]
        };
        // The outer counter does not change in the inner loop: a bound.
        assert!(ends(nest(IrCmpOp::Slt)));
        assert!(!ends(nest(IrCmpOp::Sle)));
        // The inner loop testing the OUTER counter never moves it.
        let mut stuck = nest(IrCmpOp::Slt);
        stuck[2].instructions[1] = compare(13, IrCmpOp::Slt, value(X), value(N), I32);
        assert!(!ends(stuck));
    }

    /// Two entries into one cycle: no header, so no loop to prove.
    #[test]
    fn a_cycle_that_is_not_a_natural_loop_proves_nothing() {
        let irreducible = vec![
            block(0, vec![param(N, I32), param(50, I32)], when(50, 1, 2)),
            block(
                1,
                vec![
                    phi(X, I32, vec![(constant(0, I32), 0), (value(NEXT), 2)]),
                    compare(TEST, IrCmpOp::Slt, value(X), value(N), I32),
                ],
                when(TEST, 2, 3),
            ),
            block(2, vec![add(NEXT, value(X), constant(1, I32), I32)], jump(1)),
            block(3, vec![], done()),
        ];
        assert!(!ends(irreducible));
    }

    #[test]
    fn the_counter_its_step_and_its_test_must_share_one_type() {
        let mut narrow_test = counted(IrType::I64, IrCmpOp::Slt, value(N), 1);
        narrow_test[1].instructions[1] = compare(TEST, IrCmpOp::Slt, value(X), value(N), I32);
        assert!(!ends(narrow_test));
        let mut wide_step = counted(I32, IrCmpOp::Slt, value(N), 1);
        wide_step[2].instructions[0] = add(NEXT, value(X), constant(1, IrType::I64), IrType::I64);
        assert!(!ends(wide_step));
        // Narrow counters are computed promoted and pointers may wrap.
        for ty in [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::Ptr,
        ] {
            assert!(!ends(counted(ty, IrCmpOp::Ne, value(N), 1)), "{ty:?}");
        }
        assert!(!ends(counted(IrType::I128, IrCmpOp::Ne, value(N), 1)));
        // A signed predicate on `U32` folds in the unsigned order.
        assert!(!ends(counted(U32, IrCmpOp::Slt, value(N), 1)));
        assert!(!ends(counted(U32, IrCmpOp::Sgt, constant(5, U32), -1)));
    }

    #[test]
    fn a_loop_without_a_counted_exit_does_not_end() {
        // `for (;;) {}`
        assert!(!ends(vec![
            block(0, vec![], jump(1)),
            block(1, vec![], jump(1))
        ]));
        // `while (*p) p++;` -- the test reads nothing that counts.
        let scans = vec![
            block(0, vec![param(50, I32)], jump(1)),
            block(1, vec![], when(50, 1, 2)),
            block(2, vec![], done()),
        ];
        assert!(!ends(scans));
        // A switch is not an exit test.
        let mut dispatched = counted(I32, IrCmpOp::Slt, value(N), 1);
        dispatched[1].terminator = Terminator::Switch {
            val: value(X),
            cases: vec![(5, BlockId(3))],
            default: BlockId(2),
            ty: I32,
        };
        assert!(!ends(dispatched));
        // Both arms stay in the loop, or both leave it.
        let mut stays = counted(I32, IrCmpOp::Slt, value(N), 1);
        stays[1].terminator = when(TEST, 2, 2);
        assert!(!ends(stays));
    }

    /// An asm goto can leave the block ahead of its exit test.
    #[test]
    fn an_asm_goto_proves_nothing() {
        let mut jumps = counted(I32, IrCmpOp::Slt, value(N), 1);
        jumps[1].instructions.push(Instruction::InlineAsm {
            template: "jmp %l0".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("again".into(), BlockId(2))],
            input_symbols: vec![],
            seg_overrides: vec![],
        });
        assert!(!ends(jumps));
    }

    /// A value defined twice is not in SSA form: no instruction is *the*
    /// definition of the counter.
    #[test]
    fn a_value_defined_twice_proves_nothing() {
        let mut redefined = counted(I32, IrCmpOp::Slt, value(N), 1);
        redefined[2]
            .instructions
            .push(add(NEXT, value(X), constant(1, I32), I32));
        assert!(!ends(redefined));
    }

    /// Blocks the entry cannot reach are no part of any execution: neither
    /// their cycles nor their edges into a live loop.
    #[test]
    fn dead_blocks_decide_nothing() {
        let mut blocks = counted(I32, IrCmpOp::Slt, value(N), 1);
        // A dead cycle, and a dead block that jumps to the live header
        // without advancing the counter.
        blocks.push(block(7, vec![], jump(7)));
        blocks.push(block(8, vec![], jump(1)));
        if let Instruction::Phi { incoming, .. } = &mut blocks[1].instructions[0] {
            incoming.push((constant(0, I32), BlockId(8)));
        }
        assert!(ends(blocks));
    }

    #[test]
    fn exit_arithmetic_at_the_ends_of_both_orders() {
        let ring = Ring::of(I32).unwrap();
        // Unit steps: any strict or unequal test, no bound needed.
        for op in [
            IrCmpOp::Slt,
            IrCmpOp::Sgt,
            IrCmpOp::Ult,
            IrCmpOp::Ugt,
            IrCmpOp::Ne,
        ] {
            assert!(ring.exit_is_reached(op, 1, None), "{op:?}");
            assert!(ring.exit_is_reached(op, u64::MAX, None), "{op:?} down");
        }
        for op in [IrCmpOp::Sle, IrCmpOp::Sge, IrCmpOp::Ule, IrCmpOp::Uge] {
            assert!(!ring.exit_is_reached(op, 1, None), "{op:?}");
        }
        // The largest stride of a 32-bit ring is 2^31: half of it.
        let half = 1u64 << 31;
        assert!(ring.exit_is_reached(IrCmpOp::Ult, half, Some(i64::from(i32::MAX) + 1)));
        assert!(!ring.exit_is_reached(IrCmpOp::Ult, half, Some(i64::from(i32::MAX) + 2)));
        assert!(ring.exit_is_reached(IrCmpOp::Slt, half, Some(0)));
        assert!(!ring.exit_is_reached(IrCmpOp::Slt, half, Some(1)));
        assert!(ring.exit_is_reached(IrCmpOp::Eq, half, None));
        // A constant is read modulo the width: -1 is the unsigned maximum
        // and the signed value just below zero.
        assert!(!ring.exit_is_reached(IrCmpOp::Ule, 1, Some(-1)));
        assert!(ring.exit_is_reached(IrCmpOp::Sle, 1, Some(-1)));
        assert!(ring.exit_is_reached(IrCmpOp::Ule, 1, Some(0xffff_fffe)));
        assert!(!ring.exit_is_reached(IrCmpOp::Ule, 1, Some(0xffff_ffff)));
        // 64 bits: the arithmetic must not overflow at either end.
        let wide = Ring::of(IrType::U64).unwrap();
        assert!(wide.exit_is_reached(IrCmpOp::Ult, 1, Some(-1)));
        assert!(!wide.exit_is_reached(IrCmpOp::Ule, 1, Some(-1)));
        assert!(wide.exit_is_reached(IrCmpOp::Ugt, 1, Some(0)));
        assert!(!wide.exit_is_reached(IrCmpOp::Uge, 1, Some(0)));
        assert!(wide.exit_is_reached(IrCmpOp::Sge, 1, Some(0)));
        assert!(!wide.exit_is_reached(IrCmpOp::Sge, 1, Some(i64::MIN)));
        assert!(!wide.exit_is_reached(IrCmpOp::Sle, 1, Some(i64::MAX)));
        assert!(wide.exit_is_reached(IrCmpOp::Sle, 1, Some(i64::MAX - 1)));
        assert!(wide.exit_is_reached(IrCmpOp::Slt, 1u64 << 63, Some(0)));
        assert!(!wide.exit_is_reached(IrCmpOp::Slt, 1u64 << 63, Some(1)));
        assert!(wide.exit_is_reached(IrCmpOp::Eq, 1u64 << 63, None));
    }

    /// The table, checked by running every loop of a ring small enough to do
    /// so.  With the bound known the answer is exact, not merely safe; with
    /// the bound known only to be fixed it is "for every bound".
    #[test]
    fn on_a_small_ring_the_answer_is_what_running_the_loop_shows() {
        const BITS: u32 = 5;
        const SIZE: u32 = 1 << BITS;
        let ring = Ring {
            bits: BITS,
            unsigned_only: false,
        };
        let signed = |x: u32| (x ^ (SIZE / 2)) as i32 - (SIZE / 2) as i32;
        let holds = |op: IrCmpOp, x: u32, b: u32| match op {
            IrCmpOp::Eq => x == b,
            IrCmpOp::Ne => x != b,
            IrCmpOp::Slt => signed(x) < signed(b),
            IrCmpOp::Sle => signed(x) <= signed(b),
            IrCmpOp::Sgt => signed(x) > signed(b),
            IrCmpOp::Sge => signed(x) >= signed(b),
            IrCmpOp::Ult => x < b,
            IrCmpOp::Ule => x <= b,
            IrCmpOp::Ugt => x > b,
            IrCmpOp::Uge => x >= b,
        };
        // Whether the loop is left when entered with `start`: `SIZE`
        // iterations are a full period of any counter.
        let leaves = |op: IrCmpOp, start: u32, step: u32, bound: u32| {
            let mut x = start;
            (0..SIZE).any(|_| {
                let left = !holds(op, x, bound);
                x = (x + step) % SIZE;
                left
            })
        };
        let ops = [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
            IrCmpOp::Ult,
            IrCmpOp::Ule,
            IrCmpOp::Ugt,
            IrCmpOp::Uge,
        ];
        let (mut proven, mut declined) = (0u32, 0u32);
        for op in ops {
            for step in 1..SIZE {
                let mut for_every_bound = true;
                for bound in 0..SIZE {
                    let always = (0..SIZE).all(|start| leaves(op, start, step, bound));
                    assert_eq!(
                        ring.exit_is_reached(op, u64::from(step), Some(i64::from(bound))),
                        always,
                        "{op:?} step {step} bound {bound}"
                    );
                    for_every_bound &= always;
                    proven += u32::from(always);
                    declined += u32::from(!always);
                }
                assert_eq!(
                    ring.exit_is_reached(op, u64::from(step), None),
                    for_every_bound,
                    "{op:?} step {step}, any bound"
                );
            }
        }
        // Both answers occur, so neither assertion above is vacuous.  Of the
        // 10 * 31 * 32 loops, 480 of the declined ones test `!=` with an even
        // step (15 steps, any bound: a counter of the wrong parity steps over
        // it), and none tests `==`.
        assert_eq!((proven, declined), (8924, 996));
    }
}
