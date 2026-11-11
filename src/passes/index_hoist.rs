//! Hoist the loop-invariant addend of an array index into the base pointer.
//!
//! # Why
//!
//! A flattened index `a[inv + j]` -- `o[i + j]` in a schoolbook multiply,
//! `c[i * n + j]` in any matrix kernel written over a flat array -- reaches
//! the optimizer as
//!
//! ```text
//!   t = add i32 inv, j          ; `inv` does not change in the loop
//!   c = sext t to i64
//!   s = shl c, 3
//!   g = gep base, s
//! ```
//!
//! The index of the access is the *sum*, so the passes that make an indexed
//! loop cheap never see a plain `base[j]`: the induction-variable widener
//! declines a counter that is added to a non-constant, the vectorizer and
//! the idiom recognisers do not match the address, and the x86-64 backend
//! re-emits `leal (inv, j), t; movslq t, t` for every access of every
//! iteration while holding `base`, `inv` and `t` in three registers.
//!
//! GCC and Clang split the address the other way round, and so does this
//! pass:
//!
//! ```text
//!   preheader:   gb = gep base, (sext inv) << 3     ; once per loop entry
//!   loop:        g  = gep gb,   (sext j)   << 3     ; a plain indexed access
//! ```
//!
//! After it the counter is used as an index only, so `iv_widen` makes it
//! pointer-width and the access folds to `(gb, j, 8)`.  On the bignum kernel
//! `tests/bench/k_bn_mul128.c` this is the difference between a 21- and a
//! 12-instruction inner step; `gb` replaces `base` and `inv` in the loop, so
//! the loop usually needs one register *fewer*, not one more.
//!
//! # What is rewritten
//!
//! A `GetElementPtr` inside a loop `L` whose base is invariant in `L` and
//! whose offset is, innermost first,
//!
//! * a sum `add a, b` of two values, exactly one of which is invariant in
//!   `L` -- either a pointer-width add, or a 32-bit signed add;
//! * for the 32-bit sum, its sign extension to pointer width;
//! * optionally a scale by a constant (`shl x, k` or `mul x, m`) in pointer
//!   width.
//!
//! The invariant part is built once at the end of the loop's preheader; the
//! variant part is rebuilt immediately in front of the access.  The old
//! chain is left to die, and is removed here when nothing else reads it so
//! that no stale narrow use of the counter survives to stop the widener.
//!
//! # Soundness
//!
//! Pointer arithmetic in the IR is arithmetic modulo 2^64, and a constant
//! scale distributes over it, so
//!
//! ```text
//!   base + ((x + y) * m)  ==  (base + x * m) + y * m        (mod 2^64)
//! ```
//!
//! holds for every `x`, `y`: the pointer-width form needs no precondition.
//!
//! The 32-bit form additionally uses `sext(x + y) == sext(x) + sext(y)`,
//! which holds exactly when the 32-bit signed addition does not overflow.
//! Signed overflow is undefined (C17 6.5/5), so the identity holds on every
//! execution the program defines.  This is the rule `iv_widen` (its theorem
//! 2) and the backend's index peeling (`scale_ring_is_linear`) already rely
//! on, and it is gated the same way: under `-fwrapv`/`-fno-strict-overflow`
//! (`PassPolicy::wrap_signed_overflow`) the 32-bit form is not touched.  An
//! unsigned 32-bit sum wraps by definition and is never split.
//!
//! No instruction is moved: the hoisted operands are pure integer and
//! address arithmetic on values that already dominate the preheader, so
//! computing them when the loop body does not run cannot fault, and no load
//! or store changes position.  The rewritten address is the same address,
//! so alias and dependence reasoning sees what the source
//! `T *gb = base + inv; ... gb[j]` would have shown it.
//!
//! # When it does not fire
//!
//! * No unique outside predecessor of the header to hold the hoisted code,
//!   or that predecessor is itself inside a sibling loop (the code would run
//!   once per iteration of *that* loop).
//! * The sign extension or the scale has another reader: the old chain
//!   would stay and the loop would gain work instead of losing it.
//! * The loop would need more than [`MAX_BASES_PER_LOOP`] hoisted bases
//!   *and* more of them than the bases and addends they replace: every one
//!   is a pointer that stays live across the loop (see [`within_budget`]).
//! * A constant addend (`a[j + 1]`): the widener's closure and the backend's
//!   displacement folding already handle it without a second base.
//!
//! # Scope and placement
//!
//! x86-64, `-O2` and above, not `-Os`/`-Oz`; the first optimizer iteration,
//! before unrolling and vectorization so that every later loop pass works on
//! the split form.  `CCC_DISABLE_PASSES=index_hoist` turns it off.  The
//! other targets are not enabled because the result was only measured on
//! x86-64 (see `engineering/REDTEAM-2026-11-11-I128-TYPEOF.md`).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::analysis::{CfgAnalysis, DominanceChecker};
use crate::ir::instruction::{Instruction, Operand, Value};
use crate::ir::reexports::{IrBinOp, IrFunction};
use crate::passes::loop_analysis::{
    NaturalLoop, find_natural_loops, merge_loops_by_header, pred_in_foreign_loop,
};
use crate::passes::use_def::UseDefInfo;

/// Hoisted bases a loop may always receive.  Each is a pointer that stays
/// live across the whole loop.  Beyond this number the loop is rewritten
/// only when the split cannot cost a register -- see [`within_budget`].
const MAX_BASES_PER_LOOP: usize = 6;

/// Rounds per function.  A second round catches a base that the first round
/// hoisted into the preheader of an inner loop and that has an invariant
/// addend of its own in the enclosing loop (`a[i * n + k + j]`).
const MAX_ROUNDS: usize = 4;

/// Run the pass on one function.  Returns the number of addresses rewritten.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut total = 0;
    // Hoisted bases granted so far, per loop header.  Block indices are
    // stable across rounds: the pass creates and removes no block.
    let mut granted: FxHashMap<usize, usize> = FxHashMap::default();
    for _ in 0..MAX_ROUNDS {
        let rewritten = run_round(func, &mut granted);
        if rewritten == 0 {
            break;
        }
        total += rewritten;
    }
    total
}

fn is_pointer_width_int(ty: IrType) -> bool {
    matches!(ty, IrType::I64 | IrType::U64)
}

/// A constant scale in pointer width: `x << k` or `x * m`.
#[derive(Clone, Copy)]
struct Scale {
    op: IrBinOp,
    /// The constant operand, reused verbatim in both halves.
    factor: Operand,
    /// Its value: the deduplication key.
    value: i64,
    const_is_lhs: bool,
    ty: IrType,
}

impl Scale {
    fn key(&self) -> (IrBinOp, i64, bool, IrType) {
        (self.op, self.value, self.const_is_lhs, self.ty)
    }

    fn apply(&self, dest: Value, index: Value) -> Instruction {
        let index = Operand::Value(index);
        let (lhs, rhs) = if self.const_is_lhs {
            (self.factor, index)
        } else {
            (index, self.factor)
        };
        Instruction::BinOp {
            dest,
            op: self.op,
            lhs,
            rhs,
            ty: self.ty,
        }
    }
}

/// The index expression of one access, taken apart.
#[derive(Clone, Copy)]
struct Shape {
    scale: Option<Scale>,
    /// `Some(to_ty)` when the sum is a 32-bit signed add that is
    /// sign-extended to `to_ty`.
    widen: Option<IrType>,
    invariant: Value,
    variant: Value,
    /// The scaled value and the extended value of the old chain, when the
    /// shape has them.  Both must have no reader left after the rewrite.
    scaled: Option<Value>,
    extended: Option<Value>,
    /// The sum itself.  It may keep other readers.
    sum: Value,
}

type ShapeKey = (Option<(IrBinOp, i64, bool, IrType)>, Option<IrType>);

impl Shape {
    fn key(&self) -> ShapeKey {
        (self.scale.map(|s| s.key()), self.widen)
    }
}

/// One address to rewrite.
struct Candidate {
    block: usize,
    index: usize,
    base: Value,
    /// The `ty` of the access's `GetElementPtr`, copied to the hoisted one.
    ty: IrType,
    shape: Shape,
}

/// What identifies one hoisted base: the original base, the invariant
/// addend, how the addend is extended and scaled, and the address type.
type BaseKey = (Value, Value, ShapeKey, IrType);

impl Candidate {
    fn base_key(&self) -> BaseKey {
        (self.base, self.shape.invariant, self.shape.key(), self.ty)
    }
}

/// Everything one loop asks for.
struct LoopPlan {
    preheader: usize,
    candidates: Vec<Candidate>,
}

/// The distinct hoisted bases a set of candidates needs.
fn distinct_bases(candidates: &[Candidate]) -> usize {
    let keys: FxHashSet<BaseKey> = candidates.iter().map(Candidate::base_key).collect();
    keys.len()
}

/// What the analysis of one loop may ask about a value.
struct LoopView<'a> {
    func: &'a IrFunction,
    uses: &'a UseDefInfo,
    dom: &'a DominanceChecker,
    param_types: &'a FxHashMap<u32, IrType>,
    body: &'a FxHashSet<usize>,
    preheader: usize,
}

impl LoopView<'_> {
    fn defined_in_loop(&self, v: Value) -> bool {
        self.uses
            .def_of(v.0)
            .block()
            .is_some_and(|b| self.body.contains(&(b as usize)))
    }

    /// The defining instruction of `v` when it sits inside the loop.
    fn loop_def(&self, v: Value) -> Option<&Instruction> {
        if self.defined_in_loop(v) {
            self.uses.def_inst(v.0, self.func)
        } else {
            None
        }
    }

    /// `v` is available at the end of the preheader: a parameter, or a value
    /// defined outside the loop in a block that dominates the preheader (a
    /// definition *in* the preheader precedes anything appended to it).
    ///
    /// For a value read inside the loop this follows from SSA dominance; it
    /// is checked rather than assumed so that malformed input (a value with
    /// no definition, an unreachable definition) declines instead of
    /// producing a use before its definition.
    fn hoistable(&self, v: Value) -> bool {
        let def = self.uses.def_of(v.0);
        if def.is_parameter() {
            // `param_types` holds the parameters of the entry block, which
            // dominates every block a loop analysis can reach.
            return self.param_types.contains_key(&v.0);
        }
        match def.block() {
            Some(b) => {
                let b = b as usize;
                !self.body.contains(&b)
                    && (b == self.preheader || self.dom.dominates(b, self.preheader))
            }
            None => false,
        }
    }

    fn value_type(&self, v: Value) -> Option<IrType> {
        if self.uses.def_of(v.0).is_parameter() {
            return self.param_types.get(&v.0).copied();
        }
        self.uses.def_inst(v.0, self.func)?.result_type()
    }

    /// Take the offset of an access apart, or `None` when it is not a sum of
    /// one invariant and one variant value under the accepted wrappers.
    fn shape_of(&self, offset: Value) -> Option<Shape> {
        let mut cur = offset;

        let mut scale = None;
        let mut scaled = None;
        if let Some(Instruction::BinOp {
            op, lhs, rhs, ty, ..
        }) = self.loop_def(cur)
            && is_pointer_width_int(*ty)
        {
            let found = match (op, lhs, rhs) {
                (IrBinOp::Shl, Operand::Value(x), Operand::Const(k)) => k
                    .to_i64()
                    .filter(|k| (0..64).contains(k))
                    .map(|k| (*x, *rhs, k, false)),
                (IrBinOp::Mul, Operand::Value(x), Operand::Const(m)) => {
                    m.to_i64().map(|m| (*x, *rhs, m, false))
                }
                (IrBinOp::Mul, Operand::Const(m), Operand::Value(x)) => {
                    m.to_i64().map(|m| (*x, *lhs, m, true))
                }
                _ => None,
            };
            if let Some((inner, factor, value, const_is_lhs)) = found {
                scale = Some(Scale {
                    op: *op,
                    factor,
                    value,
                    const_is_lhs,
                    ty: *ty,
                });
                scaled = Some(cur);
                cur = inner;
            }
        }

        let mut widen = None;
        let mut extended = None;
        if let Some(Instruction::Cast {
            src: Operand::Value(x),
            from_ty: IrType::I32,
            to_ty,
            ..
        }) = self.loop_def(cur)
            && is_pointer_width_int(*to_ty)
        {
            // The identity below is the signed no-overflow rule.
            if self.func.pass_policy.wrap_signed_overflow {
                return None;
            }
            widen = Some(*to_ty);
            extended = Some(cur);
            cur = *x;
        }

        let Some(Instruction::BinOp {
            op: IrBinOp::Add,
            lhs: Operand::Value(a),
            rhs: Operand::Value(b),
            ty,
            ..
        }) = self.loop_def(cur)
        else {
            return None;
        };
        let operand_ok = |v: Value| match (widen, self.value_type(v)) {
            (Some(_), Some(IrType::I32)) => true,
            (None, Some(t)) => is_pointer_width_int(t),
            _ => false,
        };
        let sum_ok = match widen {
            Some(_) => *ty == IrType::I32,
            None => is_pointer_width_int(*ty),
        };
        if !sum_ok || !operand_ok(*a) || !operand_ok(*b) || a == b {
            return None;
        }
        let (invariant, variant) = if self.hoistable(*a) && self.defined_in_loop(*b) {
            (*a, *b)
        } else if self.hoistable(*b) && self.defined_in_loop(*a) {
            (*b, *a)
        } else {
            return None;
        };
        Some(Shape {
            scale,
            widen,
            invariant,
            variant,
            scaled,
            extended,
            sum: cur,
        })
    }
}

/// The accesses of one loop worth rewriting, in program order.
fn plan_loop(
    view: &LoopView<'_>,
    blocks: &[usize],
    claimed: &FxHashSet<(usize, usize)>,
    granted: usize,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    for &block in blocks {
        for (index, inst) in view.func.blocks[block].instructions.iter().enumerate() {
            let Instruction::GetElementPtr {
                base,
                offset: Operand::Value(offset),
                ty,
                ..
            } = inst
            else {
                continue;
            };
            if claimed.contains(&(block, index)) || !view.hoistable(*base) {
                continue;
            }
            if let Some(shape) = view.shape_of(*offset) {
                found.push(Candidate {
                    block,
                    index,
                    base: *base,
                    ty: *ty,
                    shape,
                });
            }
        }
    }
    drop_unprofitable(view.uses, &mut found);
    if !within_budget(&found, granted) {
        found.clear();
    }
    found
}

/// Keep only the candidates whose scaled and extended values are read by
/// nothing but the rewritten chain: those instructions then die, and the
/// loop body loses the addition without gaining anything.  A candidate that
/// goes takes its sites out of the allowed set, so repeat until stable.
fn drop_unprofitable(uses: &UseDefInfo, found: &mut Vec<Candidate>) {
    loop {
        let mut allowed: FxHashSet<(u32, u32)> = FxHashSet::default();
        for c in found.iter() {
            allowed.insert((c.block as u32, c.index as u32));
            for v in [c.shape.scaled, c.shape.extended].into_iter().flatten() {
                if let Some(site) = uses.def_of(v.0).as_instruction() {
                    allowed.insert(site);
                }
            }
        }
        let before = found.len();
        found.retain(|c| {
            [c.shape.scaled, c.shape.extended]
                .into_iter()
                .flatten()
                .all(|v| {
                    uses.uses_of(v.0).iter().all(|site| {
                        !site.is_terminator() && allowed.contains(&(site.block_idx, site.inst_idx))
                    })
                })
        });
        if found.len() == before {
            return;
        }
    }
}

/// Whether the loop can afford the hoisted bases its candidates need, given
/// the `granted` ones it received in earlier rounds.
///
/// The split replaces the values the addresses keep live across the loop --
/// each original base and each invariant addend -- by one hoisted base per
/// distinct (base, addend, shape).  It is accepted when
///
/// * the hoisted bases are no more numerous than the bases and addends they
///   replace (seven arrays indexed by one `off + j` trade eight live values
///   for seven), or
/// * the loop ends up with at most [`MAX_BASES_PER_LOOP`] of them.
///
/// Otherwise the whole loop is left alone.  Rewriting only some of its
/// accesses is the worst outcome: the hoisted bases, the original bases and
/// the addends would all stay live.
fn within_budget(found: &[Candidate], granted: usize) -> bool {
    let hoisted = distinct_bases(found);
    if granted + hoisted <= MAX_BASES_PER_LOOP {
        return true;
    }
    let bases: FxHashSet<Value> = found.iter().map(|c| c.base).collect();
    let addends: FxHashSet<Value> = found.iter().map(|c| c.shape.invariant).collect();
    hoisted <= bases.len() + addends.len()
}

/// One round over every loop of the function, innermost first.
fn run_round(func: &mut IrFunction, granted: &mut FxHashMap<usize, usize>) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    let plans = plan_function(func, granted);
    if plans.is_empty() {
        return 0;
    }
    apply(func, plans)
}

fn plan_function(func: &IrFunction, granted: &mut FxHashMap<usize, usize>) -> Vec<LoopPlan> {
    let cfg = CfgAnalysis::build(func);
    let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if raw.is_empty() {
        return Vec::new();
    }
    let mut loops = merge_loops_by_header(raw);
    // Innermost first, so an access is claimed by the innermost loop in
    // which its index has an invariant addend.
    loops.sort_by_key(|lp| (lp.body.len(), lp.header));
    let uses = UseDefInfo::build(func);
    let dom = DominanceChecker::new(cfg.num_blocks, &cfg.idom);
    // Parameters are materialized in the entry block; one found anywhere
    // else is not treated as available in a preheader.
    let param_types: FxHashMap<u32, IrType> = func.blocks[0]
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            Instruction::ParamRef { dest, ty, .. } => Some((dest.0, *ty)),
            _ => None,
        })
        .collect();

    let mut plans = Vec::new();
    let mut claimed: FxHashSet<(usize, usize)> = FxHashSet::default();
    for lp in &loops {
        let Some(preheader) = preheader_of(lp, &cfg, &loops) else {
            continue;
        };
        let view = LoopView {
            func,
            uses: &uses,
            dom: &dom,
            param_types: &param_types,
            body: &lp.body,
            preheader,
        };
        let mut blocks: Vec<usize> = lp.body.iter().copied().collect();
        blocks.sort_unstable();
        let already = granted.get(&lp.header).copied().unwrap_or(0);
        let candidates = plan_loop(&view, &blocks, &claimed, already);
        if candidates.is_empty() {
            continue;
        }
        *granted.entry(lp.header).or_default() += distinct_bases(&candidates);
        claimed.extend(candidates.iter().map(|c| (c.block, c.index)));
        plans.push(LoopPlan {
            preheader,
            candidates,
        });
    }
    plans
}

/// The block that receives the hoisted code: the unique predecessor of the
/// header outside the loop, unless it belongs to a sibling loop.
fn preheader_of(lp: &NaturalLoop, cfg: &CfgAnalysis, loops: &[NaturalLoop]) -> Option<usize> {
    let preheader = lp.find_preheader(&cfg.preds)?;
    if pred_in_foreign_loop(preheader, lp.header, loops) {
        return None;
    }
    Some(preheader)
}

/// Fresh value ids for one rewrite.
struct Ids {
    next: u32,
}

impl Ids {
    fn fresh(&mut self) -> Value {
        let v = Value(self.next);
        self.next += 1;
        v
    }
}

/// `[sext] [scale] value`, appended to `out`; returns the operand that
/// carries the result.
fn build_index(ids: &mut Ids, shape: &Shape, value: Value, out: &mut Vec<Instruction>) -> Value {
    let mut cur = value;
    if let Some(to_ty) = shape.widen {
        let dest = ids.fresh();
        out.push(Instruction::Cast {
            dest,
            src: Operand::Value(cur),
            from_ty: IrType::I32,
            to_ty,
        });
        cur = dest;
    }
    if let Some(scale) = shape.scale {
        let dest = ids.fresh();
        out.push(scale.apply(dest, cur));
        cur = dest;
    }
    cur
}

fn apply(func: &mut IrFunction, plans: Vec<LoopPlan>) -> usize {
    let mut ids = Ids {
        next: func.next_value_id,
    };
    // Code for the end of each preheader, in creation order.
    let mut hoisted: Vec<(usize, Vec<Instruction>)> = Vec::new();
    // Per block: (index of the access, instructions to put in front of it,
    // its new base, its new offset).
    let mut edits: FxHashMap<usize, Vec<(usize, Vec<Instruction>, Value, Value)>> =
        FxHashMap::default();
    let mut dead: FxHashSet<u32> = FxHashSet::default();
    let mut rewritten = 0;

    for plan in &plans {
        let mut bases: FxHashMap<BaseKey, Value> = FxHashMap::default();
        // A variant index built earlier in the same block dominates every
        // later access of that block and is shared with it.
        let mut indexes: FxHashMap<(usize, Value, ShapeKey), Value> = FxHashMap::default();
        let mut preheader_code = Vec::new();
        for c in &plan.candidates {
            let shape = &c.shape;
            let base = *bases.entry(c.base_key()).or_insert_with(|| {
                let offset = build_index(&mut ids, shape, shape.invariant, &mut preheader_code);
                let dest = ids.fresh();
                preheader_code.push(Instruction::GetElementPtr {
                    dest,
                    base: c.base,
                    offset: Operand::Value(offset),
                    ty: c.ty,
                });
                dest
            });
            let mut front = Vec::new();
            let offset = *indexes
                .entry((c.block, shape.variant, shape.key()))
                .or_insert_with(|| build_index(&mut ids, shape, shape.variant, &mut front));
            edits
                .entry(c.block)
                .or_default()
                .push((c.index, front, base, offset));
            dead.extend(
                [shape.scaled, shape.extended, Some(shape.sum)]
                    .into_iter()
                    .flatten()
                    .map(|v| v.0),
            );
            rewritten += 1;
        }
        hoisted.push((plan.preheader, preheader_code));
    }

    // In-loop edits first, highest index first so that the recorded
    // positions stay valid; the preheader code goes to block ends afterwards
    // and cannot shift any of them.
    for (block, mut list) in edits {
        list.sort_by_key(|edit| std::cmp::Reverse(edit.0));
        let body = &mut func.blocks[block];
        for (index, front, new_base, new_offset) in list {
            if let Instruction::GetElementPtr { base, offset, .. } = &mut body.instructions[index] {
                *base = new_base;
                *offset = Operand::Value(new_offset);
            }
            let span = body.span_at(index);
            body.insert_insts(index, front, span);
        }
    }
    for (preheader, code) in hoisted {
        let body = &mut func.blocks[preheader];
        let span = body
            .source_spans
            .last()
            .copied()
            .unwrap_or_else(Span::dummy);
        body.extend_insts(code, span);
    }
    func.next_value_id = ids.next;

    remove_dead(func, dead);
    rewritten
}

/// Delete the instructions defining `candidates` that nothing reads any
/// more.  Readers are recounted on the rewritten function rather than
/// inferred from the plan, and a deletion can free the next link of a
/// chain, so iterate.
fn remove_dead(func: &mut IrFunction, mut candidates: FxHashSet<u32>) {
    while !candidates.is_empty() {
        let mut read: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                inst.for_each_used_value(|v| {
                    if candidates.contains(&v) {
                        read.insert(v);
                    }
                });
            }
            block.terminator.for_each_used_value(|v| {
                if candidates.contains(&v) {
                    read.insert(v);
                }
            });
        }
        let mut removed = false;
        for block in &mut func.blocks {
            for index in (0..block.instructions.len()).rev() {
                let Some(dest) = block.instructions[index].dest() else {
                    continue;
                };
                if candidates.contains(&dest.0) && !read.contains(&dest.0) {
                    block.remove_inst(index);
                    candidates.remove(&dest.0);
                    removed = true;
                }
            }
        }
        if !removed {
            return;
        }
    }
}

#[cfg(test)]
#[path = "index_hoist_tests.rs"]
mod tests;
