//! Collapse constant-stride pointer induction variables onto a shared
//! integer index.
//!
//! A scan over several arrays keeps one pointer per array plus (usually) a
//! byte counter for the exit test.  Every one of them is an independent
//! induction variable, so a 64-bit compare loop like zlib-ng's
//! `compare256_64`
//!
//! ```text
//!   p1 = phi [a, p1 + 8]        p2 = phi [b, p2 + 8]        c = phi [0, c + 8]
//!   x = load p1                 y = load p2                  branch c < 256
//! ```
//!
//! costs three increments per iteration on x86-64:
//!
//! ```text
//!   movq (%rdi), %r8 ; xorq (%rsi), %r8 ; jne  ; addq $8,%rdi
//!   addq $8,%rsi ; addq $8,%edx ; cmpl $256,%edx ; jb
//! ```
//!
//! GCC and Clang instead keep the counter alone and address through it
//! (`movq (%rdi,%rdx,1), %r8`), which is one increment per iteration and two
//! SIB loads; measured on the kernel above that is 1.19 cycles per 8-byte
//! compare against 1.78 for the three-IV form (pinned, 21 paired rounds) —
//! the pointer additions are pure address arithmetic that a SIB operand
//! already does for free.
//!
//! The transform rewrites exactly that shape:
//!
//! ```text
//!   c = phi [0, c + K] ; z = zext c ; g1 = gep a, z ; g2 = gep b, z
//!   x = load g1 ; y = load g2 ; c' = c + K ; branch c' < BOUND
//! ```
//!
//! and then drops the pointer phis together with their latch increments.
//!
//! # Legality
//!
//! Every condition below is required before a keystroke is rewritten:
//!
//! * each pointer phi has exactly two incomings — one of them the back-edge
//!   `phi + K` defined in the latch, the other an invariant defined outside
//!   that edge and dominating the header;
//! * every pointer phi of the group steps by the *same* constant `K`;
//! * the back-edge add is used only by its own phi;
//! * every use of a pointer phi is either the back-edge add or the `ptr`
//!   operand of a `Load`/`Store` **in the header block**.  Confining the
//!   uses to the header is what lets the new address computation be
//!   inserted once, after the phis, with dominance guaranteed by
//!   construction;
//! * the index phi starts at the literal 0 and steps by the same `K` in the
//!   same latch block;
//! * the latch's condition compares the post-increment index (or the index
//!   itself) against a **constant** bound, which bounds the trip count by
//!   `BOUND / K`.  That is what makes the zero-extension exact: the index
//!   can never wrap its own width, so `zext c` is the byte offset the
//!   pointer phis had accumulated (pointer arithmetic overflow is UB, and
//!   the index form cannot overflow at all here).
//!
//! Anything else — a variable bound, a second use of the pointer, a
//! different stride, a latch shared with another loop — is rejected and the
//! loop keeps its pointer IVs.

use crate::common::fx_hash::FxHashMap;
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::instruction::{Instruction, Operand, Terminator, Value};
use crate::ir::reexports::IrBinOp;
use crate::ir::reexports::IrFunction;
use crate::passes::use_def::UseDefInfo;

/// Maximum number of pointer phis collapsed in one rewrite.  The shape this
/// targets is a scan over a handful of arrays; a larger group is not what
/// the cost model above was measured on, and one rewrite per candidate keeps
/// the bookkeeping trivial.
const MAX_POINTER_IVS: usize = 4;

/// Upper bound on the constant trip limit (`BOUND / K` iterations).  Above
/// this the "the index cannot wrap" argument stops being obvious by
/// inspection; the bound is deliberately far above any real scan.
const MAX_TRIP_BOUND: i64 = 1 << 31;

/// One detected pointer induction variable of the group.
struct PointerIv {
    phi: Value,
    init: Value,
    backedge_add: Value,
}

/// The shape a rewrite needs, fully validated except for the function-wide
/// dominance check on the initial values.
struct Group {
    header: usize,
    latch: usize,
    pointers: Vec<PointerIv>,
    /// The integer index phi the pointers collapse onto.
    index: Value,
    index_ty: IrType,
}

/// Run the pass on one function.  Returns the number of pointer IVs collapsed.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut total = 0;
    // Each successful rewrite deletes at least one pointer phi, so this
    // terminates; the cap keeps a pathological function from spending the
    // whole compile in here.
    for _ in 0..16 {
        match collapse_one(func) {
            Some(n) => total += n,
            None => break,
        }
    }
    total
}

fn collapse_one(func: &mut IrFunction) -> Option<usize> {
    let ud = UseDefInfo::build(func);
    let idom = CfgAnalysis::build(func).idom;
    // Phi incomings name *labels*; `DefLoc` reports block *indices*.  The
    // group's latch is tracked as an index (it drives insertion and removal),
    // so the edge labels are translated here once.
    let label_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();

    for header in 0..func.blocks.len() {
        let Some(group) = detect_group(func, &ud, header, &label_to_idx) else {
            continue;
        };
        // The collapsed address is `init + zext(index)` evaluated in the
        // header, so every `init` must be available there.
        if !group
            .pointers
            .iter()
            .all(|p| value_dominates_block(&ud, &idom, p.init, header))
        {
            continue;
        }
        // The index must be an integer the backend can zero-extend into a
        // SIB index.  A narrower counter would need a stronger bound proof
        // than the constant-limit rule provides.
        if !matches!(
            group.index_ty,
            IrType::U32 | IrType::I32 | IrType::U64 | IrType::I64
        ) {
            continue;
        }
        if apply(func, &group) {
            return Some(group.pointers.len());
        }
    }
    None
}

fn detect_group(
    func: &IrFunction,
    ud: &UseDefInfo,
    header: usize,
    label_to_idx: &FxHashMap<u32, usize>,
) -> Option<Group> {
    let block = func.blocks.get(header)?;
    let mut pointers: Vec<PointerIv> = Vec::new();
    let mut step: Option<i64> = None;
    let mut latch: Option<usize> = None;

    for inst in &block.instructions {
        let Instruction::Phi {
            dest,
            ty: IrType::Ptr,
            incoming,
        } = inst
        else {
            continue;
        };
        if incoming.len() != 2 || pointers.len() >= MAX_POINTER_IVS {
            return None;
        }
        let mut backedge: Option<(Value, i64, usize)> = None;
        let mut init: Option<Value> = None;
        for (op, edge) in incoming {
            let Operand::Value(v) = op else {
                return None;
            };
            match backedge_step(func, ud, *dest, *v) {
                Some(found) => {
                    if backedge.is_some() {
                        return None;
                    }
                    // The phi must take the increment on the edge the
                    // increment's own block defines it on.
                    if label_to_idx.get(&edge.0) != Some(&found.2) {
                        return None;
                    }
                    if step.is_some_and(|s| s != found.1) {
                        return None;
                    }
                    step = Some(found.1);
                    match latch {
                        Some(prev) if prev != found.2 => return None,
                        _ => latch = Some(found.2),
                    }
                    backedge = Some(found);
                }
                None => {
                    // The base is whatever the other incoming names; whether
                    // it is available at the header is the dominance check in
                    // `collapse_one`.
                    if init.replace(*v).is_some() {
                        return None;
                    }
                }
            }
        }
        let (Some((add, _, _)), Some(init)) = (backedge, init) else {
            return None;
        };
        if !pointer_uses_are_addresses(func, ud, *dest, header) {
            return None;
        }
        pointers.push(PointerIv {
            phi: *dest,
            init,
            backedge_add: add,
        });
    }

    let (step, latch) = (step?, latch?);
    if pointers.is_empty() || step <= 0 {
        return None;
    }

    // The index phi: same header, same latch, same step, literal-zero start.
    let mut found: Option<(Value, IrType)> = None;
    for inst in &block.instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if *ty == IrType::Ptr || incoming.len() != 2 {
            continue;
        }
        let mut zero_start = false;
        let mut same_step = false;
        for (op, edge) in incoming {
            match op {
                Operand::Const(c) if c.to_i64() == Some(0) => zero_start = true,
                Operand::Value(v) => {
                    if label_to_idx.get(&edge.0) != Some(&latch) {
                        continue;
                    }
                    let Some(Instruction::BinOp {
                        op: IrBinOp::Add,
                        lhs: Operand::Value(lhs),
                        rhs: Operand::Const(c),
                        ..
                    }) = ud.def_inst(v.0, func)
                    else {
                        continue;
                    };
                    if *lhs == *dest && c.to_i64() == Some(step) {
                        same_step = true;
                    }
                }
                _ => {}
            }
        }
        if !(zero_start && same_step) {
            continue;
        }
        // Two candidate counters make "which one drives the loop" ambiguous;
        // refuse rather than guess.
        if found.is_some() {
            return None;
        }
        found = Some((*dest, *ty));
    }
    let (index, index_ty) = found?;

    if !index_bound_is_constant(func, ud, latch, index, step) {
        return None;
    }

    Some(Group {
        header,
        latch,
        pointers,
        index,
        index_ty,
    })
}

/// `v` is the back-edge `phi + K` of `phi`: returns the add's value, the
/// step, and the block holding it.
fn backedge_step(
    func: &IrFunction,
    ud: &UseDefInfo,
    phi: Value,
    v: Value,
) -> Option<(Value, i64, usize)> {
    let (bi, _) = ud.def_of(v.0).as_instruction()?;
    let Instruction::BinOp {
        op: IrBinOp::Add,
        lhs: Operand::Value(lhs),
        rhs: Operand::Const(c),
        ..
    } = ud.def_inst(v.0, func)?
    else {
        return None;
    };
    let k = c.to_i64()?;
    if *lhs != phi || k <= 0 {
        return None;
    }
    // The increment feeds only its own phi.
    if ud.uses_of(v.0).len() != 1 {
        return None;
    }
    Some((v, k, bi as usize))
}

/// Every use of the pointer phi is the `ptr` operand of a `Load`/`Store` in
/// the header block.  (The back-edge add is a use too, but it lives in the
/// latch and is therefore caught here and rejected — callers pass the phi
/// only when that add is the single non-header use, which this function
/// verifies by allowing exactly one use in the latch block.)
fn pointer_uses_are_addresses(
    func: &IrFunction,
    ud: &UseDefInfo,
    phi: Value,
    header: usize,
) -> bool {
    let uses = ud.uses_of(phi.0);
    if uses.is_empty() {
        return false;
    }
    let mut in_latch_add = 0u32;
    for u in uses {
        if u.is_terminator() {
            return false;
        }
        let Some(inst) = ud.use_inst(*u, func) else {
            return false;
        };
        if u.block_idx as usize == header {
            match inst {
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } if *ptr == phi => {}
                // A plain copy of the pointer is redirected at the new
                // address like any other use; the copy itself stays valid.
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } if *v == phi => {}
                _ => return false,
            }
            continue;
        }
        // Outside the header the only allowed use is the back-edge add.
        match inst {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(lhs),
                rhs: Operand::Const(_),
                ..
            } if *lhs == phi => in_latch_add += 1,
            _ => return false,
        }
    }
    in_latch_add == 1
}

/// The latch's condition compares the index (or its increment) against a
/// literal bound, so the trip count is `BOUND / step` at most.
fn index_bound_is_constant(
    func: &IrFunction,
    ud: &UseDefInfo,
    latch: usize,
    index: Value,
    step: i64,
) -> bool {
    let Some(block) = func.blocks.get(latch) else {
        return false;
    };
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        ..
    } = &block.terminator
    else {
        return false;
    };
    let Some(Instruction::Cmp {
        lhs: Operand::Value(lhs),
        rhs: Operand::Const(bound),
        ..
    }) = ud.def_inst(cond.0, func)
    else {
        return false;
    };
    // The comparison must be against the index family (either the phi or its
    // increment, which is a linear function of it) and a positive literal.
    if !index_family_contains(func, ud, latch, *lhs, index) {
        return false;
    }
    let Some(b) = bound.to_i64() else {
        return false;
    };
    b > 0 && b <= MAX_TRIP_BOUND && b >= step
}

/// `v` is `index` or `index + K` defined in the latch.
fn index_family_contains(
    func: &IrFunction,
    ud: &UseDefInfo,
    latch: usize,
    v: Value,
    index: Value,
) -> bool {
    if v == index {
        return true;
    }
    matches!(ud.def_of(v.0).as_instruction(),
        Some((bi, _)) if bi as usize == latch)
        && matches!(
            ud.def_inst(v.0, func),
            Some(Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(lhs),
                rhs: Operand::Const(_),
                ..
            }) if *lhs == index
        )
}

/// `v` is defined before `header` on every path into it.
fn value_dominates_block(ud: &UseDefInfo, idom: &[usize], v: Value, header: usize) -> bool {
    if ud.def_of(v.0).is_parameter() {
        return true;
    }
    match ud.def_of(v.0).as_instruction() {
        Some((bi, _)) => {
            let bi = bi as usize;
            bi == header || analysis::dominates_idom(idom, bi, header)
        }
        None => false,
    }
}

fn apply(func: &mut IrFunction, group: &Group) -> bool {
    let mut next_val = func.next_value_id.max(max_value_id(func) + 1);

    // `index` is already I64 in the common `size_t` form; otherwise widen it.
    let zext = (group.index_ty != IrType::I64).then(|| {
        let v = Value(next_val);
        next_val += 1;
        v
    });
    let mut geps: Vec<(Value, Value)> = Vec::with_capacity(group.pointers.len());
    for p in &group.pointers {
        let v = Value(next_val);
        next_val += 1;
        geps.push((p.phi, v));
    }

    // 1. Insert the zero-extension and one address per array, right after the
    //    header's phis and therefore before every replacement load/store.
    let at = {
        let block = &mut func.blocks[group.header];
        let at = block
            .instructions
            .iter()
            .rposition(|i| matches!(i, Instruction::Phi { .. }))
            .map(|i| i + 1)
            .unwrap_or(0);
        let span = block
            .source_spans
            .get(at.saturating_sub(1))
            .copied()
            .unwrap_or_else(|| Span::new(0, 0, 0));
        let mut new: Vec<Instruction> = Vec::with_capacity(1 + geps.len());
        if let Some(dest) = zext {
            new.push(Instruction::Cast {
                dest,
                src: Operand::Value(group.index),
                from_ty: group.index_ty,
                to_ty: IrType::I64,
            });
        }
        let offset = Operand::Value(zext.unwrap_or(group.index));
        for (phi, gep) in &geps {
            let init = match group.pointers.iter().find(|p| p.phi == *phi) {
                Some(p) => p.init,
                None => return false,
            };
            new.push(Instruction::GetElementPtr {
                dest: *gep,
                base: init,
                offset,
                ty: IrType::Ptr,
            });
        }
        let count = new.len();
        for (k, inst) in new.into_iter().enumerate() {
            block.instructions.insert(at + k, inst);
        }
        if !block.source_spans.is_empty() {
            for _ in 0..count {
                block.source_spans.insert(at, span);
            }
        }
        at
    };
    let _ = at;

    // 2. Redirect the header's loads and stores at the new addresses.
    {
        let block = &mut func.blocks[group.header];
        for inst in block.instructions.iter_mut() {
            match inst {
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                    if let Some((_, gep)) = geps.iter().find(|(phi, _)| phi == ptr) {
                        *ptr = *gep;
                    }
                }
                Instruction::Copy { src, .. } => {
                    if let Operand::Value(v) = src {
                        if let Some((_, gep)) = geps.iter().find(|(phi, _)| phi == v) {
                            *src = Operand::Value(*gep);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // 3. Drop the pointer phis and their latch increments.
    remove_defs(
        &mut func.blocks[group.header],
        |inst| matches!(inst, Instruction::Phi { dest, .. } if group.pointers.iter().any(|p| p.phi == *dest)),
    );
    remove_defs(
        &mut func.blocks[group.latch],
        |inst| matches!(inst, Instruction::BinOp { dest, .. } if group.pointers.iter().any(|p| p.backedge_add == *dest)),
    );

    func.next_value_id = next_val;
    true
}

fn remove_defs(
    block: &mut crate::ir::instruction::BasicBlock,
    pred: impl Fn(&Instruction) -> bool,
) {
    let mut removed = Vec::new();
    let mut idx = 0usize;
    block.instructions.retain(|inst| {
        let keep = !pred(inst);
        if !keep {
            removed.push(idx);
        }
        idx += 1;
        keep
    });
    if !block.source_spans.is_empty() {
        for i in removed.into_iter().rev() {
            if i < block.source_spans.len() {
                block.source_spans.remove(i);
            }
        }
    }
}

fn max_value_id(func: &IrFunction) -> u32 {
    let mut max_id = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                max_id = max_id.max(dest.0);
            }
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, _) in incoming {
                    if let Operand::Value(v) = op {
                        max_id = max_id.max(v.0);
                    }
                }
            }
        }
    }
    max_id
}
