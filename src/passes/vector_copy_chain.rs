//! Collapse the 16/32-byte vector HOME copy chain produced by aggregate
//! intrinsic loads.
//!
//! LCCC has no IR vector *value* type for explicit SIMD: `__m128i` / `__m256i`
//! are aggregates with a stack HOME.  A direct unaligned load therefore
//! lowers to three instructions even when the loaded bytes are immediately
//! consumed by a single SIMD operation:
//!
//! ```text
//!   %s = alloca 16
//!   %h = alloca 16
//!   intrinsic Loaddqu   dest_ptr=%s, args=[%p]   ; read %p -> scratch home
//!   intrinsic VecLoadI64x2  dest=%v, args=[%s, 0] ; home -> SSA vector value
//!   intrinsic VecStoreI64x2 dest_ptr=%h, args=[%v, %h, 0] ; value -> result home
//!   intrinsic CmpEpi8Mask128 args=[%h, %h2, $imm]  ; consumer reads the HOME
//! ```
//!
//! The middle pair is a pure whole-object copy: `VecLoad*` reads exactly
//! `width` bytes at offset 0 and `VecStore*` writes exactly `width` bytes at
//! offset 0.  Rewriting the load's `dest_ptr` from `%s` to `%h` and deleting
//! the pair produces
//!
//! ```text
//!   intrinsic Loaddqu dest_ptr=%h, args=[%p]
//!   intrinsic CmpEpi8Mask128 args=[%h, %h2, $imm]
//! ```
//!
//! This does NOT move the memory read (it stays at the original load site),
//! so it needs NO alias or volatility proof for `%p`.  It only moves the
//! *write* to `%h` two instructions earlier, which is unobservable because
//! the three instructions are required to be contiguous in one basic block
//! and the two deleted instructions are the only ones in between.
//!
//! The real payoff is that this shape is exactly what
//! `vector_temp_promotion::fuse_vector_loads` needs: with the load writing
//! directly to the final home, its audited alias proof can forward `%p`
//! into the consumer's vector operand and delete the load altogether.  On
//! the 485-case AVX-512VL mask-compare matrix that removes the 128/256-bit
//! home roundtrip (four register copies + a `leaq` per compare), while the
//! 512-bit cases never had the middle pair at all.
//!
//! Correctness is deliberately fail-closed and narrow:
//! * one basic block, three STRICTLY CONTIGUOUS instructions;
//! * both homes are non-volatile `Alloca`s of EXACTLY the load width;
//! * the load is a single-argument direct full-width load, the `VecLoad*`
//!   is a whole-register load of the SAME width at offset 0, the `VecStore*`
//!   is a whole-register store of the SAME width at offset 0 (a partial
//!   store such as `VecStoreI32x4Pair` refuses);
//! * the scratch home is mentioned exactly twice (load `dest_ptr`,
//!   `VecLoad*` base) and the SSA vector value exactly once (`VecStore*`
//!   source) in the whole function — no cross-block reader can be stranded;
//! * the load's own SSA `dest` is kept, so a future lowering that exposes it
//!   still sees the same definition;
//! * the now-dead scratch `Alloca` is removed only when it has zero
//!   remaining mentions, with `retain_insts` keeping `-g` spans in lockstep.
//!
//! Pass name for `CCC_DISABLE_PASSES`: `veccopychain`.
//! Explainability is STRUCTURAL, not an environment switch: every refusal
//! below carries its reason in a source comment and is exercised by
//! `tests::refuses_when_widths_offsets_or_shapes_are_wrong` /
//! `tests::rejects_same_home_semantic_volatile_and_cross_block_shapes`.
//! A compiler pass must not read the process environment on its hot path
//! (`check_env_test_hygiene.sh` ratchets that count down) and shipped sources
//! may not contain `eprintln!` (`check_param_home_contracts.py` check 8), so
//! there is deliberately NO debug trace here: it would cost a branch per
//! candidate and weaken both invariants for a developer convenience.

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_value_use_in_instruction,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, IrModule, Operand, Value};

use super::vector_temp_promotion::{
    direct_vector_load_width, whole_vector_load, whole_vector_store_width,
};

/// Whole-register `VecLoad*` width.  `whole_vector_load` proves the load
/// reads an unaligned-safe whole register; this table supplies the exact
/// byte count so a 32-byte load feeding a 16-byte store refuses.
fn vec_load_width(op: IntrinsicOp) -> Option<usize> {
    // An explicit guard (rather than `Some(match ...).filter(...)`) keeps the
    // width table the ONLY source of byte counts and makes the fail-closed
    // order obvious: no shape proof => no width, no eager match evaluation.
    if !whole_vector_load(op) {
        return None;
    }
    use IntrinsicOp as O;
    Some(match op {
        O::VecLoadI32x4
        | O::VecLoadF32x4
        | O::VecLoadF64x2
        | O::VecLoadI64x2
        | O::VecLoadI16x8
        | O::VecLoadI8x16 => 16,
        O::VecLoadI32x8
        | O::VecLoadF32x8
        | O::VecLoadF64x4
        | O::VecLoadI64x4
        | O::VecLoadI8x32
        | O::VecLoadI16x16 => 32,
        _ => return None,
    })
}

/// Count every MENTION of each value: intrinsic `args`, `dest_ptr`,
/// scalar/GPR uses and terminator operands.  A value's own `Alloca`
/// definition is not a mention.  This is the same two-walk convention
/// `dse::eliminate_dead_private_slot_loads` uses (the operand walk sees
/// `Intrinsic.args`, the use walk sees `dest_ptr`).
fn mention_counts(func: &IrFunction) -> FxHashMap<u32, usize> {
    let mut counts: FxHashMap<u32, usize> = FxHashMap::default();
    let mut bump = |id: u32| *counts.entry(id).or_insert(0usize) += 1;
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    bump(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| bump(v.0));
        }
        block.terminator.for_each_used_value(|v: u32| bump(v));
    }
    counts
}

/// Non-volatile private vector homes, keyed by value id.
fn homes(func: &IrFunction) -> FxHashMap<u32, usize> {
    let mut out = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                out.insert(dest.0, *size);
            }
        }
    }
    out
}

#[inline]
fn zero_const(op: &Operand) -> bool {
    matches!(op, Operand::Const(IrConst::I64(0)))
}

struct Plan {
    load: usize,
    vec_load: usize,
    vec_store: usize,
    new_dest_ptr: Value,
    dead_alloca: Option<u32>,
}

pub(crate) fn collapse_vector_copy_chains(module: &mut IrModule) -> usize {
    module
        .functions
        .iter_mut()
        .filter(|f| !f.is_declaration && !f.blocks.is_empty())
        .map(collapse_in_function)
        .sum()
}

fn collapse_in_function(func: &mut IrFunction) -> usize {
    let homes = homes(func);
    if homes.is_empty() {
        return 0;
    }
    let counts = mention_counts(func);
    let mut plans: Vec<Vec<Plan>> = (0..func.blocks.len()).map(|_| Vec::new()).collect();
    let mut claimed: FxHashSet<u32> = FxHashSet::default();

    for (bi, block) in func.blocks.iter().enumerate() {
        let insts = &block.instructions;
        if insts.len() < 3 {
            // refuse: block has fewer than three instructions
            continue;
        }
        for i in 0..insts.len() - 2 {
            let (
                Instruction::Intrinsic {
                    op: load_op,
                    dest_ptr: Some(scratch),
                    args: load_args,
                    ..
                },
                Instruction::Intrinsic {
                    dest: Some(vec_val),
                    op: load_vec_op,
                    dest_ptr: None,
                    args: vl_args,
                    ..
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: store_vec_op,
                    dest_ptr: Some(final_home),
                    args: vs_args,
                    ..
                },
            ) = (&insts[i], &insts[i + 1], &insts[i + 2])
            else {
                // refuse: triple is not load/VecLoad/VecStore with the right dests
                continue;
            };
            // A single source pointer: no offset, mask or width-changing load.
            if load_args.len() != 1 {
                // refuse: load has more than one source argument
                continue;
            }
            let Some(width) = direct_vector_load_width(*load_op) else {
                // refuse: load is not a direct full-width vector load
                continue;
            };
            if vec_load_width(*load_vec_op) != Some(width)
                || whole_vector_store_width(*store_vec_op) != Some(width)
            {
                // refuse: VecLoad/VecStore widths or whole-register shape mismatch
                continue;
            }
            if homes.get(&scratch.0) != Some(&width) || homes.get(&final_home.0) != Some(&width) {
                // refuse: homes are missing, volatile or not exactly the load width
                continue;
            }
            if scratch.0 == final_home.0 {
                // refuse: scratch and final home are the same alloca
                continue;
            }
            // Unique-def / unique-use guards: a value id defined or read by
            // anything else must not be rewired.
            if counts.get(&scratch.0).copied() != Some(2)
                || counts.get(&vec_val.0).copied() != Some(1)
            {
                // refuse: scratch/SSA value has a non-unique mention count
                continue;
            }
            // Exact whole-object, offset-0 copy.  `VecLoad*` args are
            // [base, byte-offset]; `VecStore*` args are [value, ptr, offset]
            // and `dest_ptr` MUST be that same ptr.
            if vl_args.len() != 2
                || !matches!(&vl_args[0], Operand::Value(v) if v.0 == scratch.0)
                || !zero_const(&vl_args[1])
            {
                // refuse: VecLoad base/arity/offset is not an exact offset-0 read
                continue;
            }
            if vs_args.len() != 3
                || !matches!(&vs_args[0], Operand::Value(v) if v.0 == vec_val.0)
                || !matches!(&vs_args[1], Operand::Value(v) if v.0 == final_home.0)
                || !zero_const(&vs_args[2])
            {
                // refuse: VecStore value/ptr/offset is not an exact offset-0 write
                continue;
            }
            // Do not let two plans share a home (a scratch of one plan can
            // never be the final home of another because of the mention
            // counts, but the guard makes that invariant explicit).
            if claimed.contains(&scratch.0) || claimed.contains(&final_home.0) {
                // refuse: a home is already claimed by another plan
                continue;
            }
            claimed.insert(scratch.0);
            claimed.insert(final_home.0);
            // The scratch Alloca is dead after the rewrite: its only two
            // mentions were the load's dest_ptr and the VecLoad base.
            let dead_alloca = func
                .blocks
                .iter()
                .any(|b| {
                    b.instructions.iter().any(|inst| {
                        matches!(inst, Instruction::Alloca { dest, .. } if dest.0 == scratch.0)
                    })
                })
                .then_some(scratch.0);
            plans[bi].push(Plan {
                load: i,
                vec_load: i + 1,
                vec_store: i + 2,
                new_dest_ptr: *final_home,
                dead_alloca,
            });
        }
    }

    let total = plans.iter().map(Vec::len).sum::<usize>();
    if total == 0 {
        return 0;
    }

    // Apply the `dest_ptr` edits FIRST, while every planned index still
    // refers to the original instruction list.
    let mut remove: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); func.blocks.len()];
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        for plan in &plans[bi] {
            if let Some(Instruction::Intrinsic { dest_ptr, .. }) =
                block.instructions.get_mut(plan.load)
            {
                *dest_ptr = Some(plan.new_dest_ptr);
            }
            remove[bi].insert(plan.vec_load);
            remove[bi].insert(plan.vec_store);
        }
    }
    // A dead scratch Alloca may live in a DIFFERENT block than the chain.
    // Only remove it when its ONLY mentions were the two rewired/deleted
    // ones (`counts == 2`), so no other block can observe its address.
    let mut dead_allocas: FxHashSet<u32> = FxHashSet::default();
    for plans_in_block in &plans {
        for plan in plans_in_block {
            if let Some(id) = plan.dead_alloca {
                if counts.get(&id).copied() == Some(2) {
                    dead_allocas.insert(id);
                }
            }
        }
    }
    // ONE stable retain per block over the ORIGINAL indices.  Doing both
    // removals in a single pass is load-bearing: an earlier index-based
    // removal would shift the `Alloca` indices out from under the second
    // pass.  `retain_insts` keeps `source_spans` / `source_inline_ids`
    // parallel so `-g` line tables stay exact.
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        let doomed = &remove[bi];
        if doomed.is_empty() && dead_allocas.is_empty() {
            continue;
        }
        let original_len = block.instructions.len();
        let mut idx = 0usize;
        block.retain_insts(|inst| {
            let mut keep = !doomed.contains(&idx);
            if let Instruction::Alloca { dest, .. } = inst {
                if dead_allocas.contains(&dest.0) {
                    keep = false;
                }
            }
            idx += 1;
            keep
        });
        debug_assert_eq!(idx, original_len, "every instruction must be visited once");
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    fn func_with(insts: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        f
    }

    fn alloca(id: u32, size: usize, volatile: bool) -> Instruction {
        Instruction::Alloca {
            dest: Value(id),
            ty: IrType::Ptr,
            size,
            align: size,
            volatile,
            semantic_volatile: false,
        }
    }

    fn gaddr(id: u32) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(id),
            name: "A".to_string(),
        }
    }

    fn load(id: u32, scratch: u32, src: u32, op: IntrinsicOp) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(id)),
            op,
            dest_ptr: Some(Value(scratch)),
            args: vec![Operand::Value(Value(src))],
        }
    }

    fn vload(id: u32, scratch: u32, op: IntrinsicOp, off: i64) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(id)),
            op,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(scratch)),
                Operand::Const(IrConst::I64(off)),
            ],
        }
    }

    fn vstore(val: u32, home: u32, ptr: u32, op: IntrinsicOp, off: i64) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(Value(home)),
            args: vec![
                Operand::Value(Value(val)),
                Operand::Value(Value(ptr)),
                Operand::Const(IrConst::I64(off)),
            ],
        }
    }

    fn consumer(a: u32, b: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(90)),
            op: IntrinsicOp::CmpEpi8Mask128,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(a)),
                Operand::Value(Value(b)),
                Operand::Const(IrConst::I64(0)),
            ],
        }
    }

    fn shape(
        width: usize,
        lop: IntrinsicOp,
        vop: IntrinsicOp,
        sop: IntrinsicOp,
    ) -> Vec<Instruction> {
        vec![
            alloca(1, width, false), // final home
            alloca(2, width, false), // scratch
            gaddr(3),                // source pointer
            load(4, 2, 3, lop),
            vload(5, 2, vop, 0),
            vstore(5, 1, 1, sop, 0),
            consumer(1, 1),
        ]
    }

    #[test]
    fn collapses_128_and_256_bit_copy_chains() {
        for (width, lop, vop, sop) in [
            (
                16,
                IntrinsicOp::Loaddqu,
                IntrinsicOp::VecLoadI64x2,
                IntrinsicOp::VecStoreI64x2,
            ),
            (
                32,
                IntrinsicOp::Loadu256,
                IntrinsicOp::VecLoadI64x4,
                IntrinsicOp::VecStoreI64x4,
            ),
        ] {
            let mut f = func_with(shape(width, lop, vop, sop));
            let mut m = IrModule::new();
            m.functions.push(f);
            assert_eq!(collapse_vector_copy_chains(&mut m), 1, "w{width}");
            f = m.functions.pop().unwrap();
            let insts = &f.blocks[0].instructions;
            assert_eq!(insts.len(), 4, "w{width}: {insts:#?}");
            assert!(matches!(
                &insts[2],
                Instruction::Intrinsic { dest_ptr: Some(Value(1)), op, .. } if *op == lop
            ));
            assert!(
                !insts
                    .iter()
                    .any(|i| matches!(i, Instruction::Alloca { dest: Value(2), .. }))
            );
        }
    }

    #[test]
    fn refuses_when_widths_offsets_or_shapes_are_wrong() {
        use IntrinsicOp as O;
        let good = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        let mut cases: Vec<(&str, Vec<Instruction>)> = Vec::new();
        // Non-zero load offset.
        let mut v = good.clone();
        if let Instruction::Intrinsic { args, .. } = &mut v[4] {
            args[1] = Operand::Const(IrConst::I64(8));
        }
        cases.push(("vecload offset", v));
        // Non-zero store offset.
        let mut v = good.clone();
        if let Instruction::Intrinsic { args, .. } = &mut v[5] {
            args[2] = Operand::Const(IrConst::I64(8));
        }
        cases.push(("vecstore offset", v));
        // Store dest_ptr differs from the ptr argument.
        let mut v = good.clone();
        if let Instruction::Intrinsic { dest_ptr, .. } = &mut v[5] {
            *dest_ptr = Some(Value(2));
        }
        cases.push(("dest_ptr mismatch", v));
        // Partial store (8 of 16 bytes).
        let mut v = good.clone();
        if let Instruction::Intrinsic { op, .. } = &mut v[5] {
            *op = O::VecStoreI32x4Pair;
        }
        cases.push(("partial store", v));
        // Mixed widths: 32-byte load, 16-byte VecLoad.
        let mut v = good.clone();
        if let Instruction::Intrinsic { op, .. } = &mut v[3] {
            *op = O::Loadu256;
        }
        if let Instruction::Alloca { size, .. } = &mut v[0] {
            *size = 32;
        }
        if let Instruction::Alloca { size, .. } = &mut v[1] {
            *size = 32;
        }
        cases.push(("mixed width", v));
        // Volatile homes.
        let mut v = good.clone();
        if let Instruction::Alloca { volatile, .. } = &mut v[1] {
            *volatile = true;
        }
        cases.push(("volatile scratch", v));
        // A second reader of the scratch home.
        let mut v = good.clone();
        v.push(Instruction::Load {
            dest: Value(91),
            ptr: Value(2),
            ty: IrType::I64,
            volatile: false,
            seg_override: AddressSpace::Default,
        });
        cases.push(("extra scratch reader", v));
        // A second reader of the SSA vector value.
        let mut v = good.clone();
        v.push(vstore(5, 1, 1, O::VecStoreI64x2, 0));
        cases.push(("extra value reader", v));
        // Non-contiguous: an unrelated instruction between the load and VecLoad.
        let mut v = good.clone();
        v.insert(
            4,
            Instruction::Copy {
                dest: Value(92),
                src: Operand::Const(IrConst::I64(0)),
            },
        );
        cases.push(("non-contiguous", v));

        for (name, insts) in cases {
            let mut m = IrModule::new();
            m.functions.push(func_with(insts));
            assert_eq!(collapse_vector_copy_chains(&mut m), 0, "{name} must refuse");
        }
    }

    #[test]
    fn never_rewrites_a_load_whose_source_is_not_a_single_pointer() {
        use IntrinsicOp as O;
        let mut insts = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        if let Instruction::Intrinsic { args, .. } = &mut insts[3] {
            args.push(Operand::Const(IrConst::I64(0)));
        }
        let mut m = IrModule::new();
        m.functions.push(func_with(insts));
        assert_eq!(collapse_vector_copy_chains(&mut m), 0);
    }

    #[test]
    fn rejects_same_home_semantic_volatile_and_cross_block_shapes() {
        use IntrinsicOp as O;
        let good = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);

        // Scratch and final home are the SAME alloca: the middle pair is not
        // a copy, it is a read-modify-write of one object.
        let mut v = good.clone();
        if let Instruction::Intrinsic { dest_ptr, .. } = &mut v[5] {
            *dest_ptr = Some(Value(2));
        }
        if let Instruction::Intrinsic { args, .. } = &mut v[5] {
            args[1] = Operand::Value(Value(2));
        }
        let mut m = IrModule::new();
        m.functions.push(func_with(v));
        assert_eq!(collapse_vector_copy_chains(&mut m), 0, "same home");

        // A `semantic_volatile` alloca is observable even when its `volatile`
        // flag is false (e.g. a VLA / setjmp-visible home).
        let mut v = good.clone();
        if let Instruction::Alloca {
            semantic_volatile, ..
        } = &mut v[0]
        {
            *semantic_volatile = true;
        }
        let mut m = IrModule::new();
        m.functions.push(func_with(v));
        assert_eq!(collapse_vector_copy_chains(&mut m), 0, "semantic volatile");

        // The three instructions are split across two basic blocks: the pass
        // requires STRICT contiguity and must not hoist the store.
        let mut f = IrFunction::new("t".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: good[..5].to_vec(),
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: good[5..].to_vec(),
            terminator: Terminator::Return(None),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        let mut m = IrModule::new();
        m.functions.push(f);
        assert_eq!(collapse_vector_copy_chains(&mut m), 0, "cross block");
    }

    #[test]
    fn two_independent_chains_both_collapse_but_shared_homes_are_claimed_once() {
        use IntrinsicOp as O;
        // Two fully disjoint chains: both plans must apply.
        let mut disjoint = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        let mut second = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        // Rebase the second chain's value ids so nothing is shared.
        let rebase = |insts: &mut Vec<Instruction>| {
            for i in insts.iter_mut() {
                match i {
                    Instruction::Alloca { dest, .. } | Instruction::GlobalAddr { dest, .. } => {
                        dest.0 += 20
                    }
                    Instruction::Intrinsic {
                        dest,
                        dest_ptr,
                        args,
                        ..
                    } => {
                        if let Some(d) = dest {
                            d.0 += 20;
                        }
                        if let Some(d) = dest_ptr {
                            d.0 += 20;
                        }
                        for a in args.iter_mut() {
                            if let Operand::Value(x) = a {
                                x.0 += 20;
                            }
                        }
                    }
                    _ => {}
                }
            }
        };
        rebase(&mut second);
        disjoint.extend(second);
        let mut m = IrModule::new();
        m.functions.push(func_with(disjoint));
        assert_eq!(collapse_vector_copy_chains(&mut m), 2, "disjoint chains");

        // Two chains that share the FINAL home: the `claimed` guard must
        // allow exactly one, never two conflicting `dest_ptr` rewrites.
        let mut shared = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        let mut second = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        for i in second.iter_mut() {
            match i {
                Instruction::Alloca { dest, .. } if dest.0 == 1 => dest.0 = 1, // shared final
                Instruction::Alloca { dest, .. } | Instruction::GlobalAddr { dest, .. } => {
                    dest.0 += 20
                }
                Instruction::Intrinsic {
                    dest,
                    dest_ptr,
                    args,
                    ..
                } => {
                    if let Some(d) = dest {
                        d.0 += 20;
                    }
                    if let Some(d) = dest_ptr {
                        if d.0 != 1 {
                            d.0 += 20;
                        }
                    }
                    for a in args.iter_mut() {
                        if let Operand::Value(x) = a {
                            if x.0 != 1 {
                                x.0 += 20;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        shared.extend(second);
        let mut m = IrModule::new();
        m.functions.push(func_with(shared));
        let n = collapse_vector_copy_chains(&mut m);
        assert!(n <= 1, "shared final home must not produce two plans: {n}");
    }

    #[test]
    fn rewrite_keeps_debug_spans_and_inline_ids_in_lockstep() {
        use crate::common::source::Span;
        use IntrinsicOp as O;
        let insts = shape(16, O::Loaddqu, O::VecLoadI64x2, O::VecStoreI64x2);
        let n = insts.len();
        let mut f = IrFunction::new("t".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: vec![Span::dummy(); n],
            source_inline_ids: vec![0; n],
        });
        let mut m = IrModule::new();
        m.functions.push(f);
        assert_eq!(collapse_vector_copy_chains(&mut m), 1);
        let b = &m.functions[0].blocks[0];
        assert_eq!(b.source_spans.len(), b.instructions.len(), "spans");
        assert_eq!(
            b.source_inline_ids.len(),
            b.instructions.len(),
            "inline ids"
        );
        assert_eq!(
            b.instructions.len(),
            n - 3,
            "two intrinsics + dead alloca removed"
        );
    }

    #[test]
    fn empty_and_declaration_functions_are_noops() {
        let mut m = IrModule::new();
        let mut empty = IrFunction::new("e".into(), IrType::Void, vec![], false);
        empty.is_declaration = true;
        m.functions.push(empty);
        m.functions
            .push(IrFunction::new("b".into(), IrType::Void, vec![], false));
        assert_eq!(collapse_vector_copy_chains(&mut m), 0);
    }
}
