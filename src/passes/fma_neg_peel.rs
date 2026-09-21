//! Peel operand negations into the scalar FMA family selection.
//!
//! `__builtin_fma(-a, b, -c)` reaches the backend as two `Neg` instructions
//! feeding a plain [`IntrinsicOp::FmaScalarF64`], and codegen materialises
//! `vxorpd` + `vxorpd` + `vfmadd` — three instructions for what GCC lowers to
//! ONE (`vfnmsub132sd`, oracle-verified for every one of the six negation
//! spellings at -O1 and above). The negation is a sign the FMA instruction
//! family can carry itself:
//!
//! ```text
//! fma(-a, b, -c) = round((-a)·b + (-c)) = round(-(a·b) - c) = vfnmsub(a,b,c)
//! ```
//!
//! (round(-x) = -round(x) holds in every IEEE-754 rounding mode, so the
//! rewrite preserves the result VALUE exactly; NaN payload/sign stays in the
//! established implementation-defined position shared with GCC/Clang.)
//!
//! # Why the IR layer and not the peephole
//!
//! The obvious alternative — a text-level fold of `vxorpd <sign-mask>` into a
//! following FMA — needs const-pool value visibility in the peephole (the
//! `LineStore` has none), and the generation-layer alternative needs
//! gap-fusion-style register/slot clash analysis because the fused
//! instruction reads the pre-negation operand LATER than its IR liveness
//! recorded. Peeling HERE is correct by construction instead: the rewrite is
//! a pure operand substitution in phi-SSA form, so dominance gives the
//! soundness argument (the Neg's source dominates the Neg, the Neg's def
//! dominates the fma's use, dominance is transitive) and the register
//! allocator simply sees the TRUE live ranges. Every backend that lowers
//! `FmaScalarF{32,64}` gains the four signed families through its own
//! intrinsic arm — no new trait hook.
//!
//! # Rules
//!
//! * A `Neg` is absorbable only when its destination has EXACTLY ONE use —
//!   the fma argument being rewritten (the MulAddFusion discipline). A Neg
//!   read anywhere else stays materialised. Residual: `fma(-a, -a, c)` (the
//!   SAME negated value in two argument positions) has use count 2 and is
//!   left alone; GCC folds it, the shape is a pathological spelling.
//! * Only float `Neg` of the matching width (F32 for `FmaScalarF32`, F64 for
//!   `FmaScalarF64`); an integer `Neg` is never touched.
//! * Chained negations peel transitively (`fma(-(-a), b, c)` → plain
//!   `fma(a, b, c)`), each level under the same single-use rule.
//! * Two product-side negations cancel: the instruction stays the PLAIN
//!   intrinsic with the peeled operands (no degenerate `Signed(false,false)`
//!   is ever created).
//! * NOT contract-gated: `__builtin_fma` carries C99 single-rounding
//!   semantics regardless of `-ffp-contract`, and the peel changes no
//!   arithmetic, only the spelling of an already-fused operation.
//! * Placement: LAST in the pipeline, after every vectorizer (their SLP
//!   pattern matchers must not see the new variants) and before
//!   `eliminate_phis` (the rewrite reasons in phi-SSA dominance). At -O1 and
//!   above — GCC keeps the negations materialised at -O0 and so do we.
//! * Kill switches: `CCC_NO_FMA_NEG_PEEL=1`, `CCC_DISABLE_PASSES=fmanegpeel`.
//!
//! The pass is a no-op unless the target lowers the signed families —
//! the same `has_fma3` signal that admits the fma libcall fold (x86-64 with
//! FMA3, i686 with -mfma, AArch64; all three ISAs carry all four family
//! spellings).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::module::IrModule;
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// Gate: the target must lower all four signed families (see module doc).
/// Read once per module, not per function.
fn target_lowers_signed_fma() -> bool {
    super::simplify::has_fma3()
}

pub(crate) fn run(module: &mut IrModule) -> usize {
    peel_module(module, target_lowers_signed_fma())
}

/// Gate-injectable core (the capability static is process-global; unit tests
/// must not race their siblings by flipping it — see `TWO_BLOCK_UNROLL_ENABLED`
/// for the established pattern).
pub(crate) fn peel_module(module: &mut IrModule, target_lowers_signed_families: bool) -> usize {
    if !target_lowers_signed_families {
        return 0;
    }
    let mut total = 0usize;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        total += peel_function(func);
    }
    total
}

/// One fma site's peel decision, computed against the pre-rewrite IR.
struct Peel {
    /// (block index, instruction index) of the intrinsic.
    block: usize,
    inst: usize,
    /// Replacement operands (pre-negation sources).
    args: Vec<Operand>,
    negate_product: bool,
    negate_addend: bool,
    /// Value ids of the absorbed Neg destinations (deleted in the sweep).
    absorbed: Vec<u32>,
}

fn peel_function(func: &mut IrFunction) -> usize {
    // ---- Pass 1 (shared, immutable): use counts and float-Neg sites. ----
    // Uses are counted over instructions AND terminators (the uniform
    // liveness discipline) so a Neg feeding a terminator can never be
    // misjudged as single-use.
    let mut max_id: u32 = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                max_id = max_id.max(dest.0);
            }
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    max_id = max_id.max(v.0);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                max_id = max_id.max(v.0);
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                max_id = max_id.max(v.0);
            }
        });
    }
    let mut uses = vec![0u32; max_id as usize + 1];
    let count_use = |id: u32, uses: &mut [u32]| {
        if (id as usize) < uses.len() {
            uses[id as usize] += 1;
        }
    };
    // (block, index) of every float Neg, keyed by destination value id.
    let mut float_negs: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    count_use(v.0, &mut uses);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                count_use(v.0, &mut uses);
            });
            if let Instruction::UnaryOp {
                dest,
                op: crate::ir::reexports::IrUnaryOp::Neg,
                ty,
                ..
            } = inst
            {
                if matches!(ty, IrType::F32 | IrType::F64) {
                    float_negs.insert(dest.0, (bi, ii));
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                count_use(v.0, &mut uses);
            }
        });
    }

    // ---- Pass 2 (decisions, still immutable): peel each fma site. ----
    let mut decisions: Vec<Peel> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic { op, args, .. } = inst else {
                continue;
            };
            let expected_ty = match op {
                IntrinsicOp::FmaScalarF32 => IrType::F32,
                IntrinsicOp::FmaScalarF64 => IrType::F64,
                _ => continue,
            };
            if args.len() != 3 {
                continue;
            }
            let mut absorbed: Vec<u32> = Vec::new();
            let mut new_args = Vec::with_capacity(3);
            let mut negate_product = false;
            let mut negate_addend = false;
            for (pos, arg) in args.iter().enumerate() {
                // Peel the (possibly chained) single-use Negs off this
                // operand. Each level flips the position's sign flag; the
                // loop terminates because each step moves to a strictly
                // earlier definition (SSA defs are acyclic).
                let mut current = arg.clone();
                loop {
                    let Operand::Value(v) = &current else { break };
                    // Copy the id out of the borrow: the loop tail reassigns
                    // `current` while the value is already consumed.
                    let vid = v.0;
                    let Some(&(nb, ni)) = float_negs.get(&vid) else {
                        break;
                    };
                    // Single-use: the ONLY reader may be this very argument
                    // slot. A Neg referenced twice (another fma argument,
                    // another instruction, a terminator) stays materialised.
                    if uses.get(vid as usize).copied().unwrap_or(0) != 1 {
                        break;
                    }
                    // Type discipline: the Neg must be the float negation of
                    // the fma's own width — an integer Neg (or a width
                    // mismatch, which the IR typing should have prevented)
                    // must never be folded into an FP sign.
                    let (src, ty_ok) = match &func.blocks[nb].instructions[ni] {
                        Instruction::UnaryOp { src, ty, .. } => (src.clone(), *ty == expected_ty),
                        _ => unreachable!("float_negs only records UnaryOp::Neg sites"),
                    };
                    if !ty_ok {
                        break;
                    }
                    current = src;
                    match pos {
                        0 | 1 => negate_product = !negate_product,
                        _ => negate_addend = !negate_addend,
                    }
                    absorbed.push(vid);
                }
                new_args.push(current);
            }
            if absorbed.is_empty() {
                continue;
            }
            decisions.push(Peel {
                block: bi,
                inst: ii,
                args: new_args,
                negate_product,
                negate_addend,
                absorbed,
            });
        }
    }
    if decisions.is_empty() {
        return 0;
    }

    // ---- Pass 3 (apply): rewrite the fma sites, then sweep the dead Negs. ----
    let mut absorbed_ids: FxHashSet<u32> = FxHashSet::default();
    for dec in &decisions {
        for &id in &dec.absorbed {
            absorbed_ids.insert(id);
        }
        if let Some(Instruction::Intrinsic { op, args, .. }) =
            func.blocks[dec.block].instructions.get_mut(dec.inst)
        {
            *args = dec.args.clone();
            if dec.negate_product || dec.negate_addend {
                *op = match op {
                    IntrinsicOp::FmaScalarF32 => {
                        IntrinsicOp::FmaScalarF32Signed(dec.negate_product, dec.negate_addend)
                    }
                    IntrinsicOp::FmaScalarF64 => {
                        IntrinsicOp::FmaScalarF64Signed(dec.negate_product, dec.negate_addend)
                    }
                    // The decision pass only records plain FmaScalar sites;
                    // another decision cannot have touched this instruction
                    // (distinct sites, applied once).
                    other => unreachable!("peel decision on non-FmaScalar intrinsic {other:?}"),
                };
            }
        }
    }
    // Sweep: every absorbed Neg's destination has no remaining use (that is
    // what single-use certified), so removal is dead-code elimination with
    // the parallel source_spans discipline (see dce::sweep_block). The keep
    // decision is computed once into a bitmap so instructions and spans walk
    // the SAME positional filter.
    let mut removed = 0usize;
    for block in &mut func.blocks {
        if !block.instructions.iter().any(|inst| {
            matches!(inst, Instruction::UnaryOp {
                dest,
                op: crate::ir::reexports::IrUnaryOp::Neg,
                ..
            } if absorbed_ids.contains(&dest.0))
        }) {
            continue;
        }
        let keep: Vec<bool> = block
            .instructions
            .iter()
            .map(|inst| {
                !matches!(inst, Instruction::UnaryOp {
                    dest,
                    op: crate::ir::reexports::IrUnaryOp::Neg,
                    ..
                } if absorbed_ids.contains(&dest.0))
            })
            .collect();
        let orig_len = block.instructions.len();
        let has_spans = !block.source_spans.is_empty() && block.source_spans.len() == orig_len;
        let mut idx = 0usize;
        block.instructions.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
        removed += orig_len - block.instructions.len();
        if has_spans {
            let mut idx = 0usize;
            block.source_spans.retain(|_| {
                let k = keep[idx];
                idx += 1;
                k
            });
        } else if !block.source_spans.is_empty() {
            // Restore the documented invariant ("empty spans == no debug
            // info for this block"), exactly as dce::sweep_block does: a
            // stale non-empty vector that no longer matches the instruction
            // list would desynchronize later debug emission.
            block.source_spans.clear();
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::{IrFunction, IrModule, IrParam};
    use crate::ir::reexports::{BasicBlock, BlockId, IrUnaryOp, Terminator};

    fn v(id: u32) -> Operand {
        Operand::Value(crate::ir::reexports::Value(id))
    }

    fn neg(dest: u32, src: u32, ty: IrType) -> Instruction {
        Instruction::UnaryOp {
            dest: crate::ir::reexports::Value(dest),
            op: IrUnaryOp::Neg,
            src: v(src),
            ty,
        }
    }

    fn fma(dest: u32, a: Operand, b: Operand, c: Operand, f64_ty: bool) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(crate::ir::reexports::Value(dest)),
            op: if f64_ty {
                IntrinsicOp::FmaScalarF64
            } else {
                IntrinsicOp::FmaScalarF32
            },
            dest_ptr: None,
            args: vec![a, b, c],
        }
    }

    fn module(instructions: Vec<Instruction>, next_id: u32) -> IrModule {
        let mut func = IrFunction::new(
            "t".to_string(),
            IrType::F64,
            vec![IrParam {
                ty: IrType::F64,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: Vec::new(),
                is_f128_sse: false,
                riscv_float_class: None,
            }],
            false,
        );
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            // Every test's fma destination is value 2; returning the LAST
            // id instead would accidentally read a Neg's destination and
            // inflate its use count (the helper's original bug — the peel
            // was right to reject those).
            terminator: Terminator::Return(Some(v(2))),
            source_spans: vec![],
        });
        func.next_value_id = next_id;
        let mut module = IrModule::new();
        module.functions.push(func);
        module
    }

    fn peel(module: &mut IrModule) -> usize {
        super::peel_module(module, true)
    }

    fn sole_inst(module: &IrModule) -> &Instruction {
        &module.functions[0].blocks[0].instructions[0]
    }

    #[test]
    fn negated_multiplier_peels_into_signed_family() {
        // fma(-a, b, c) -> FmaScalarF64Signed(true, false, [a, b, c])
        let mut m = module(
            vec![neg(1, 0, IrType::F64), fma(2, v(1), v(3), v(4), true)],
            5,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, false));
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn both_sides_negated_uses_fnmsub_family() {
        // fma(-a, b, -c) -> (true, true)
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 4, IrType::F64),
                fma(2, v(1), v(3), v(5), true),
            ],
            6,
        );
        assert_eq!(peel(&mut m), 2);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, true));
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn double_multiplier_negation_cancels_to_plain() {
        // fma(-a, -b, c) -> plain FmaScalarF64(a, b, c)
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 3, IrType::F64),
                fma(2, v(1), v(5), v(4), true),
            ],
            6,
        );
        assert_eq!(peel(&mut m), 2);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn chained_negations_peel_transitively() {
        // fma(-(-a), b, c) -> plain fma(a, b, c), both Negs dead
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 1, IrType::F64),
                fma(2, v(5), v(3), v(4), true),
            ],
            6,
        );
        assert_eq!(peel(&mut m), 2);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn non_adjacent_negations_peel() {
        // The f5 shape: negations computed early, unrelated work between.
        let unrelated = Instruction::BinOp {
            dest: crate::ir::reexports::Value(6),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(3),
            rhs: v(4),
            ty: IrType::F64,
        };
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                unrelated,
                neg(5, 4, IrType::F64),
                fma(2, v(1), v(6), v(5), true),
            ],
            7,
        );
        assert_eq!(peel(&mut m), 2);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 2);
        match &insts[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, true));
                assert_eq!(args, &vec![v(0), v(6), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn multi_use_negation_is_left_alone() {
        // %1 = -a is read by the fma AND by the trailing add: keep it.
        let reader = Instruction::BinOp {
            dest: crate::ir::reexports::Value(6),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(1),
            rhs: v(3),
            ty: IrType::F64,
        };
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                fma(2, v(1), v(3), v(4), true),
                reader,
            ],
            7,
        );
        assert_eq!(peel(&mut m), 0);
        match &m.functions[0].blocks[0].instructions[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(1), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn same_value_in_two_positions_is_not_absorbed() {
        // fma(n, n, c) with n = -a: use count 2 -> residual, untouched.
        let mut m = module(
            vec![neg(1, 0, IrType::F64), fma(2, v(1), v(1), v(4), true)],
            5,
        );
        assert_eq!(peel(&mut m), 0);
        match &m.functions[0].blocks[0].instructions[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(1), v(1), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn width_mismatch_is_never_absorbed() {
        // A Neg with the WRONG width defining an fma argument is mistyped IR
        // (the SSA typing should have prevented it); the peel must reject it
        // defensively rather than fold an integer negation into an FP sign.
        let mut m = module(
            vec![neg(1, 0, IrType::I64), fma(2, v(1), v(3), v(4), true)],
            5,
        );
        assert_eq!(peel(&mut m), 0);
        match &m.functions[0].blocks[0].instructions[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(1), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn f32_peels_with_own_width() {
        let mut m = module(
            vec![neg(1, 0, IrType::F32), fma(2, v(1), v(3), v(4), false)],
            5,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF32Signed(true, false));
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn addend_only_negation_peels() {
        // fma(a, b, -c) -> (false, true) = vfmsub family on x86.
        let mut m = module(
            vec![neg(5, 4, IrType::F64), fma(2, v(0), v(3), v(5), true)],
            6,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(false, true));
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn gate_off_is_a_no_op() {
        let mut m = module(
            vec![neg(1, 0, IrType::F64), fma(2, v(1), v(3), v(4), true)],
            5,
        );
        assert_eq!(super::peel_module(&mut m, false), 0);
        assert_eq!(m.functions[0].blocks[0].instructions.len(), 2);
    }

    #[test]
    fn source_spans_stay_aligned() {
        // Two Negs + fma with spans: the sweep must keep spans parallel.
        let mut func = IrFunction::new(
            "t".to_string(),
            IrType::F64,
            vec![IrParam {
                ty: IrType::F64,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: Vec::new(),
                is_f128_sse: false,
                riscv_float_class: None,
            }],
            false,
        );
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                neg(1, 0, IrType::F64),
                neg(5, 4, IrType::F64),
                fma(2, v(1), v(3), v(5), true),
            ],
            terminator: Terminator::Return(Some(v(2))),
            source_spans: vec![
                crate::common::source::Span::new(10, 20, 0),
                crate::common::source::Span::new(30, 40, 0),
                crate::common::source::Span::new(50, 60, 0),
            ],
        });
        func.next_value_id = 6;
        let mut module = IrModule::new();
        module.functions.push(func);
        assert_eq!(peel(&mut module), 2);
        let block = &module.functions[0].blocks[0];
        assert_eq!(block.instructions.len(), 1);
        // The surviving span must be the FMA's, not one of the Negs'.
        assert_eq!(block.source_spans.len(), 1);
        assert_eq!(
            block.source_spans[0],
            crate::common::source::Span::new(50, 60, 0)
        );
    }
}
