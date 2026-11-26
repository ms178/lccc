//! Fold a byte pointer difference through a phi of constant-offset addresses.
//!
//! SQLite's `sqlite3GetVarint` advances `p` 0..8 times in branch arms and
//! returns `(u8)(p - start) + 1`. After SSA promotion the return is a subtraction
//! of a pointer phi from `start`, although EVERY arm of that phi is either
//! `start` or `GetElementPtr(start, constant)` on x86-64. On i686, AArch64
//! and RISC-V the same lowering produces a pointer-width `Add(start, constant)`.
//! Keeping the pointer phi makes the backend compute `start+n; ...; sub start`
//! at the shared exit. An integer phi of the proven offsets avoids the
//! subtraction; a dead pointer phi no longer holds its address web live.
//!
//! This pass deliberately does NOT speculate about arbitrary pointers: only a
//! uniquely defined, acyclic tree of pointer phis/copies/constant addresses
//! rooted at the exact same SSA base is accepted. Variable offsets, a different base, phis
//! with non-pointer types, cycles, negative offsets, and offsets that do not
//! fit the subtraction type all leave the original IR unchanged. Backward
//! address steps (even when encoded as `Add(base, -n)`/GEP rather than `Sub`)
//! made a guarded host workload slower despite smaller assembly. Existing
//! pointer phis remain if they have another use. Run in SSA immediately before
//! phi elimination.
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
    sub_site: (usize, usize),
    lhs: Value,
    base: Value,
    ty: IrType,
    root: Arm,
    phis: Vec<PhiPlan>,
}

fn offset_constant(op: Operand, ty: IrType) -> Option<i64> {
    let n = match (ty, op) {
        (IrType::I64, Operand::Const(IrConst::I64(n))) => Some(n),
        (IrType::I64, Operand::Const(IrConst::I32(n))) => Some(i64::from(n)),
        (IrType::I32, Operand::Const(IrConst::I32(n))) => Some(i64::from(n)),
        (IrType::I32, Operand::Const(IrConst::I64(n))) => i32::try_from(n).ok().map(i64::from),
        _ => None,
    }?;
    // A backward arm can lower as either Add(base, -n) or GEP(base, -n),
    // not just Sub(base, n). The guarded Add/GEP host screen was slower with
    // the fold in 25/31 pairs; accepting it here bypassed the Sub negative
    // control. Keep zero/forward steps and leave backward steps untouched.
    (n >= 0).then_some(n)
}

fn materialize_constant(n: i64, ty: IrType) -> Operand {
    // Only I32/I64 subtractions enter build_plan. offset_constant has already
    // checked the I32 range; this cast therefore preserves the exact bits.
    if ty == IrType::I64 {
        Operand::Const(IrConst::I64(n))
    } else {
        debug_assert_eq!(ty, IrType::I32);
        Operand::Const(IrConst::I32(n as i32))
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
            // The non-x86 pointer lowering expresses the same byte address
            // as integer Add rather than GEP. Require the EXACT source SSA
            // base and the target's pointer width: an I32 add on an LP64
            // pointer could truncate high bits before the later subtraction.
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(base),
                rhs: offset,
                ty,
                ..
            }
            | Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: offset,
                rhs: Operand::Value(base),
                ty,
                ..
            } if *base == self.base
                && *ty == self.ty
                && *ty == crate::common::types::target_int_ir_type() =>
            {
                offset_constant(*offset, self.ty).map(Arm::Constant)
            }
            // Deliberately skip `Sub(base, constant)` backward steps here.
            // offset_constant also rejects equivalent negative Add/GEP arms:
            // both guarded three-arm variants shrank x86 code but REGRESSED
            // in separate 31-pair host screens. Keep them as negative controls
            // until a backend-aware improvement is measured.
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
    sub_site: (usize, usize),
) -> Option<Plan> {
    // The subtraction must live in the same modular ring as pointer
    // arithmetic. On ILP32 an I64 subtraction of zero-extended wrapped
    // pointer values is NOT in general equal to the I64 constant offset.
    // This is conservative even for frontends that only form such a wide
    // subtraction after a separate, explicitly representable cast.
    if ty != crate::common::types::target_int_ir_type() {
        return None;
    }
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
        sub_site,
        lhs,
        base,
        ty,
        root,
        phis: matcher.phis,
    })
}

fn materialize_plan(plan: &Plan, next_id: &mut u32, pending: &mut [Vec<Instruction>]) -> Operand {
    // The next free ID was soundly scanned ONCE before applying any plans.
    // There are at most 1,000,000 unique SSA definitions and 16 new phis
    // per subtraction, so even the worst case stays far below u32::MAX.
    // Allocate all IDs before translating incoming edges; the plan is in
    // postorder, and nested phis refer only to already-proven source phis.
    let mut replacements = FxHashMap::default();
    for p in &plan.phis {
        replacements.insert(p.original, Value(*next_id));
        *next_id += 1;
    }
    let convert = |arm: Arm| match arm {
        Arm::Constant(n) => materialize_constant(n, plan.ty),
        Arm::Phi(original) => Operand::Value(replacements[&original]),
    };
    for p in &plan.phis {
        pending[p.block].push(Instruction::Phi {
            dest: replacements[&p.original],
            ty: plan.ty,
            incoming: p
                .incoming
                .iter()
                .map(|&(arm, edge)| (convert(arm), edge))
                .collect(),
        });
    }
    convert(plan.root)
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
            } else if let Instruction::InlineAsm { outputs, .. } = inst {
                // An asm output can define a fresh SSA Value even though
                // dest() returns None. An output already defined by an earlier
                // instruction is a memory home (see the IR verifier), not a
                // second SSA definition. Record the former so a later normal
                // definition cannot silently masquerade as the only one.
                for (_, value, _) in outputs {
                    let entry = &mut defs[value.0 as usize];
                    if entry.is_none() {
                        *entry = Some((bi, ii));
                    }
                }
            }
        }
    }
    let mut plans = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(lhs),
                rhs: Operand::Value(base),
                ty: ty @ (IrType::I32 | IrType::I64),
            } = inst
                && let Some(plan) = build_plan(func, &defs, *dest, *lhs, *base, *ty, (bi, ii))
                && !plan.phis.is_empty()
            {
                plans.push(plan);
            }
        }
    }
    let count = plans.len();
    if count == 0 {
        return 0;
    }
    // All subtract sites are recorded BEFORE any insertion, so rewriting at
    // their original indices is O(1), even for a huge single exit block.
    // Phis are spliced once per block, not one quadratic Vec insert per phi.
    let mut pending: Vec<Vec<Instruction>> = (0..func.blocks.len()).map(|_| Vec::new()).collect();
    let mut roots: FxHashMap<(Value, Value, IrType), Operand> = FxHashMap::default();
    let mut next_id = next_id;
    for plan in plans {
        let root = *roots
            .entry((plan.lhs, plan.base, plan.ty))
            .or_insert_with(|| materialize_plan(&plan, &mut next_id, &mut pending));
        let inst = &mut func.blocks[plan.sub_site.0].instructions[plan.sub_site.1];
        debug_assert_eq!(inst.dest(), Some(plan.sub_dest));
        *inst = Instruction::Copy {
            dest: plan.sub_dest,
            src: root,
        };
    }
    for (block, new_phis) in func.blocks.iter_mut().zip(pending) {
        if !new_phis.is_empty() {
            let at = block
                .instructions
                .iter()
                .take_while(|i| matches!(i, Instruction::Phi { .. }))
                .count();
            block.insert_insts(at, new_phis, crate::common::source::Span::dummy());
        }
    }
    func.next_value_id = next_id;
    crate::passes::dce::eliminate_dead_code(func);
    count
}
