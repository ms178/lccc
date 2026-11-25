//! Fold a byte pointer difference through a phi of constant-offset addresses.
//!
//! SQLite's `sqlite3GetVarint` advances `p` 0..8 times in branch arms and
//! returns `(u8)(p - start) + 1`. After SSA promotion the return is a subtraction
//! of a pointer phi from `start`, although EVERY arm of that phi is either
//! `start` or `GetElementPtr(start, constant)`. Keeping the pointer phi makes
//! the backend compute `lea start+n; ...; sub start` at the shared exit. An
//! integer phi of the proven offsets is equivalent and avoids the subtraction;
//! dropping the now-dead pointer phi also lets its GEP fold into a load.
//!
//! This pass deliberately does NOT speculate about arbitrary pointers: only a
//! uniquely defined, acyclic tree of pointer phis/copies/GEPs rooted at the
//! exact same SSA base is accepted. Variable offsets, a different base, phis
//! with non-pointer types, cycles, and offsets that do not fit the subtraction
//! type all leave the original IR unchanged. Existing pointer phis remain if
//! they have another use. Run in SSA immediately before phi elimination.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

#[derive(Clone, Copy)]
enum Arm {
    Constant(i64),
    Phi(Value),
}

struct PhiPlan {
    original: Value,
    block: usize,
    incoming: Vec<(Arm, BlockId)>,
}

struct Plan {
    sub_dest: Value,
    ty: IrType,
    root: Arm,
    phis: Vec<PhiPlan>,
}

fn offset_constant(op: Operand, ty: IrType) -> Option<i64> {
    match (ty, op) {
        (IrType::I64, Operand::Const(IrConst::I64(n))) => Some(n),
        (IrType::I64, Operand::Const(IrConst::I32(n))) => Some(i64::from(n)),
        (IrType::I32, Operand::Const(IrConst::I32(n))) => Some(i64::from(n)),
        (IrType::I32, Operand::Const(IrConst::I64(n))) => i32::try_from(n).ok().map(i64::from),
        _ => None,
    }
}

fn materialize_constant(n: i64, ty: IrType) -> Operand {
    match ty {
        IrType::I64 => Operand::Const(IrConst::I64(n)),
        IrType::I32 => Operand::Const(IrConst::I32(n as i32)),
        _ => unreachable!("the matcher accepts only signed pointer-width types"),
    }
}

struct Matcher<'a> {
    func: &'a IrFunction,
    defs: &'a [Option<(usize, usize)>],
    base: Value,
    ty: IrType,
    visiting: FxHashSet<Value>,
    planned: FxHashSet<Value>,
    phis: Vec<PhiPlan>,
}

impl Matcher<'_> {
    fn arm(&mut self, value: Value, depth: usize) -> Option<Arm> {
        if value == self.base {
            return Some(Arm::Constant(0));
        }
        if depth > 16 || self.phis.len() > 16 || !self.visiting.insert(value) {
            return None;
        }
        let (bi, ii) = self.defs.get(value.0 as usize)?.as_ref().copied()?;
        let result = match &self.func.blocks[bi].instructions[ii] {
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => self.arm(*src, depth + 1),
            Instruction::GetElementPtr { base, offset, .. } if *base == self.base => {
                offset_constant(*offset, self.ty).map(Arm::Constant)
            }
            Instruction::Phi {
                ty: IrType::Ptr,
                incoming,
                ..
            } if (2..=16).contains(&incoming.len()) => {
                if self.planned.contains(&value) {
                    Some(Arm::Phi(value))
                } else {
                    // Collect ALL arms before adding a node: a failed leaf
                    // cannot leave a partial plan that later gets applied.
                    let mut mapped = Vec::with_capacity(incoming.len());
                    for &(op, pred) in incoming {
                        let Operand::Value(src) = op else { return None };
                        mapped.push((self.arm(src, depth + 1)?, pred));
                    }
                    self.planned.insert(value);
                    self.phis.push(PhiPlan {
                        original: value,
                        block: bi,
                        incoming: mapped,
                    });
                    Some(Arm::Phi(value))
                }
            }
            _ => None,
        };
        self.visiting.remove(&value);
        result
    }
}

fn build_plan(
    func: &IrFunction,
    defs: &[Option<(usize, usize)>],
    dest: Value,
    lhs: Value,
    base: Value,
    ty: IrType,
) -> Option<Plan> {
    let mut matcher = Matcher {
        func,
        defs,
        base,
        ty,
        visiting: FxHashSet::default(),
        planned: FxHashSet::default(),
        phis: Vec::new(),
    };
    let root = matcher.arm(lhs, 0)?;
    if matcher.phis.len() > 16 {
        return None;
    }
    Some(Plan {
        sub_dest: dest,
        ty,
        root,
        phis: matcher.phis,
    })
}

fn fresh_value(func: &mut IrFunction) -> Value {
    let id = func.sound_next_value_id();
    func.next_value_id = id + 1;
    Value(id)
}

fn apply(func: &mut IrFunction, plan: Plan) {
    // Phi plans are in postorder, so every nested integer phi is already
    // assigned an id when its parent needs it. Allocate before inserting;
    // the source phi and its edge labels remain untouched.
    let mut replacements = FxHashMap::default();
    for p in &plan.phis {
        replacements.insert(p.original, fresh_value(func));
    }
    let convert = |arm: Arm| match arm {
        Arm::Constant(n) => materialize_constant(n, plan.ty),
        Arm::Phi(original) => Operand::Value(replacements[&original]),
    };
    for p in plan.phis {
        let new_phi = Instruction::Phi {
            dest: replacements[&p.original],
            ty: plan.ty,
            incoming: p
                .incoming
                .into_iter()
                .map(|(arm, edge)| (convert(arm), edge))
                .collect(),
        };
        let block = &mut func.blocks[p.block];
        let at = block
            .instructions
            .iter()
            .take_while(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        block.insert_inst(at, new_phi, crate::common::source::Span::dummy());
    }
    for block in &mut func.blocks {
        if let Some(inst) = block
            .instructions
            .iter_mut()
            .find(|i| i.dest() == Some(plan.sub_dest))
        {
            *inst = Instruction::Copy {
                dest: plan.sub_dest,
                src: convert(plan.root),
            };
            return;
        }
    }
    unreachable!("a matched subtract destination must still exist");
}

/// Rewrite every proven constant-offset pointer subtraction in a function.
/// Returns the number of eliminated subtractions; unchanged IR on rejection.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.is_declaration || func.blocks.is_empty() {
        return 0;
    }
    // The real SQLite amalgamation contains many thousands of instructions;
    // skip uninteresting functions without allocating a definition table.
    if !func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    ty: IrType::I32 | IrType::I64,
                    ..
                }
            )
        })
    }) || !func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Phi {
                    ty: IrType::Ptr,
                    ..
                }
            )
        })
    }) {
        return 0;
    }
    // max_value_id() trusts next_value_id when it is nonzero. Other passes
    // can leave that cache stale, so a sound scan is required before using
    // Value IDs as indices (and before allocating replacement phis).
    let next_id = func.sound_next_value_id();
    if next_id > 1_000_000 || next_id == u32::MAX {
        return 0;
    }
    let mut defs = vec![None; next_id as usize];
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                let entry = &mut defs[dest.0 as usize];
                if entry.replace((bi, ii)).is_some() {
                    // Not in SSA: do not infer facts across different defs.
                    return 0;
                }
            }
        }
    }
    let mut plans = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(lhs),
                rhs: Operand::Value(base),
                ty: ty @ (IrType::I32 | IrType::I64),
            } = inst
            {
                if let Some(plan) = build_plan(func, &defs, *dest, *lhs, *base, *ty) {
                    if !plan.phis.is_empty() {
                        plans.push(plan);
                    }
                }
            }
        }
    }
    let count = plans.len();
    for plan in plans {
        apply(func, plan);
    }
    if count > 0 {
        crate::passes::dce::eliminate_dead_code(func);
    }
    count
}
