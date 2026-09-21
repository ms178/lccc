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
//! * A `Neg` is absorbable when EVERY use of it disappears with the
//!   rewrite: an argument read of a width-matching `FmaScalar` intrinsic
//!   (every such site peels it — see the fixpoint below), or the source
//!   read of another absorbable `Neg` (chain-internal). A Neg read by
//!   anything else (an add, a store, a phi, a terminator) stays
//!   materialised. This is the MULTI-USE rule: `fma(-a, b, -c)` twice
//!   over one shared `-a`/`-c` (the phi-diamond shape) peels BOTH sites —
//!   GCC contracts the two-use case (two `vfnmsub` where we used to keep
//!   two `vxorpd` + two `vfmadd`), and `fma(-a, -a, c)` cancels to the
//!   plain family exactly like GCC. The discipline is: the NaN-sign
//!   latitude the family absorption takes (the hardware propagates a
//!   source NaN's sign UNNEGATED through the negated families — CPU-
//!   verified, see `hw_nan` in the session record) is only taken when the
//!   materialisation actually dies; a Neg that survives for other readers
//!   keeps its exact materialised semantics.
//! * Only float `Neg` of the matching width (F32 for `FmaScalarF32`, F64 for
//!   `FmaScalarF64`); an integer `Neg` is never touched.
//! * Chained negations peel transitively (`fma(-(-a), b, c)` → plain
//!   `fma(a, b, c)`), each level under the same absorbability rule.
//! * Two product-side negations cancel: the instruction stays the PLAIN
//!   intrinsic with the peeled operands (no degenerate `Signed(false,false)`
//!   is ever created).
//! * Sites that are ALREADY `FmaScalar*Signed` peel too: the signed
//!   families compose by XOR (`Signed(np, na)` absorbing one more Neg on a
//!   product slot is `Signed(np^1, na)`), so a site the vectorizer's
//!   scalar remainder mirror already signed can still shed a materialised
//!   Neg, and a double peel that cancels normalises back to the plain
//!   spelling. This is what makes the `FmaArg` classification TRUE — every
//!   read classified as a peelable argument slot really does disappear
//!   with the rewrite — instead of coincidentally conservative (the
//!   vector main lanes pin the Neg today only because `VecMadd*` reads
//!   classify as `Other`).
//! * NOT contract-gated: `__builtin_fma` carries C99 single-rounding
//!   semantics regardless of `-ffp-contract`, and the peel changes no
//!   arithmetic, only the spelling of an already-fused operation.
//! * Placement: LAST in the pipeline, after every vectorizer (their SLP
//!   pattern matchers must not see the new variants) and before
//!   `eliminate_phis` (the rewrite reasons in phi-SSA dominance). At -O1 and
//!   above — GCC keeps the negations materialised at -O0 and so do we.
//! * Kill switches: `CCC_NO_FMA_NEG_PEEL=1`, `CCC_DISABLE_PASSES=fmanegpeel`.
//!
//! The absorbability fixpoint: a Neg may be DELETED iff
//!   (i)   none of its reads is non-peeling (an add, a store, a phi, a
//!         terminator, a width-mismatched slot — a read that survives every
//!         rewrite pins the materialisation),
//!   (ii)  every reader of it (every Neg whose `src` is this Neg) is deleted
//!         with it — a surviving outer Neg keeps its read of the inner Neg
//!         alive, so the inner one must stay materialised too, and
//!   (iii) some fma-argument slot reaches it by descending `src` links
//!         through deleted Negs — otherwise no site substitutes its read.
//! (ii) and (iii) are mutually recursive — dropping a Neg pins its source
//! (the dropped Neg is now a surviving reader of it) and shortens every
//! chain that descended through it — so the answer is the GREATEST
//! fixpoint: start from every peeling-read-only Neg and remove the ones
//! that violate (ii) or (iii) until stable. The edge directions are
//! load-bearing in BOTH directions at once: propagating badness from a
//! Neg's SOURCE (the direction this pass first shipped with) deletes the
//! inner link of a chain whose outer link survives — a dangling use that
//! reached codegen as an ICE; and propagating only from readers (the
//! one-line repair) still deletes a Neg whose surviving reader no fma
//! chain reaches. The reader closure and the site reachability must be
//! computed TOGETHER; tools/ir_shape_check.py enumerates the chain shape
//! space (4545 deduplicated shapes up to three Negs) and keeps both
//! failure modes pinned as negative controls.
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
    /// The replacement intrinsic: the site's own family with every
    /// absorbed sign flip folded in (plain seeds start from (false,
    /// false); Signed seeds XOR into their existing flags). A full cancel
    /// lands on the PLAIN spelling — no degenerate `Signed(false, false)`
    /// is ever created.
    op: IntrinsicOp,
    /// Value ids of the absorbed Neg destinations (deleted in the sweep).
    absorbed: Vec<u32>,
}

/// How one read of a float `Neg` interacts with the peel.
///
/// * `FmaArg` — an argument read of a width-matching, 3-arg `FmaScalar`
///   intrinsic: the use disappears iff the site peels it, which it will
///   for every absorbable Neg its chains walk through.
/// * `NegSrc` — the source read of another float `Neg` (a chain link):
///   the use disappears iff that outer Neg is itself absorbed.
/// * `Other` — anything else (a plain FP op, a store, a phi, a terminator,
///   a width-mismatched fma): the use survives every rewrite, pinning the
///   materialisation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NegUseKind {
    FmaArg,
    NegSrc,
    Other,
}

fn fma_arg_width(op: &IntrinsicOp) -> Option<IrType> {
    match op {
        IntrinsicOp::FmaScalarF32 | IntrinsicOp::FmaScalarF32Signed(..) => Some(IrType::F32),
        IntrinsicOp::FmaScalarF64 | IntrinsicOp::FmaScalarF64Signed(..) => Some(IrType::F64),
        _ => None,
    }
}

fn peel_function(func: &mut IrFunction) -> usize {
    // ---- Pass 1: locate every float-Neg site. ----
    // (block, index) of every float Neg, keyed by destination value id.
    // The per-use classification below is what decides absorbability; the
    // old use-COUNT fast path is subsumed by it (a single fma-arg read is
    // the trivial all-uses-absorbable case).
    let mut float_negs: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
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
    }

    // ---- Pass 1b: classify every read of every float Neg. ----
    // The classification drives the multi-use absorbability rule: a Neg
    // may be folded into family flags only when ALL its readers vanish
    // with the rewrite. Width mismatches (mistyped IR the SSA typing
    // should have prevented) classify as `Other` — the defensive reject.
    let mut neg_use_kinds: FxHashMap<u32, Vec<NegUseKind>> =
        FxHashMap::with_capacity_and_hasher(float_negs.len(), Default::default());
    let neg_ty = |vid: u32| -> Option<IrType> {
        float_negs
            .get(&vid)
            .and_then(|&(nb, ni)| match &func.blocks[nb].instructions[ni] {
                Instruction::UnaryOp { ty, .. } => Some(*ty),
                _ => None,
            })
    };
    {
        let mut classify_use = |vid: u32, kind: NegUseKind| {
            neg_use_kinds.entry(vid).or_default().push(kind);
        };
        let classify_inst = |inst: &Instruction, classify_use: &mut dyn FnMut(u32, NegUseKind)| {
            // The kind this instruction imparts to every float-Neg value
            // it reads (either as an Operand or as a bare Value use).
            let kind = match inst {
                Instruction::Intrinsic { op, args, .. }
                    if args.len() == 3 && fma_arg_width(op).is_some() =>
                {
                    // Width check per-Neg below (the arg positions share
                    // the site's width; the Neg's own ty decides).
                    NegUseKind::FmaArg
                }
                Instruction::UnaryOp {
                    op: crate::ir::reexports::IrUnaryOp::Neg,
                    ty,
                    ..
                } if matches!(ty, IrType::F32 | IrType::F64) => NegUseKind::NegSrc,
                _ => NegUseKind::Other,
            };
            let fma_width = match inst {
                Instruction::Intrinsic { op, args, .. } if args.len() == 3 => fma_arg_width(op),
                _ => None,
            };
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if let Some(nt) = neg_ty(v.0) {
                        let k = match kind {
                            NegUseKind::FmaArg => match fma_width {
                                Some(w) if w == nt => NegUseKind::FmaArg,
                                // Width-mismatched fma slot (mistyped IR):
                                // the defensive reject, same as the peel's
                                // own type discipline.
                                _ => NegUseKind::Other,
                            },
                            // A chain link's width is fixed by the Neg's own
                            // typing; NegSrc and Other pass through.
                            other => other,
                        };
                        classify_use(v.0, k);
                    }
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                if let Some(_nt) = neg_ty(v.0) {
                    // Intrinsic dest_ptr and pointer-ish uses: a float Neg
                    // can only appear here in mistyped IR; pin it.
                    classify_use(v.0, NegUseKind::Other);
                }
            });
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                classify_inst(inst, &mut classify_use);
            }
            crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    if neg_ty(v.0).is_some() {
                        classify_use(v.0, NegUseKind::Other);
                    }
                }
            });
        }
    }

    // ---- Pass 1c: the absorbability fixpoint (greatest fixpoint). ----
    // A Neg may be deleted iff
    //   (i)   none of its reads is `Other` (a read that survives every
    //         rewrite pins the materialisation),
    //   (ii)  every reader of it — every Neg whose `src` is this Neg — is
    //         deleted with it (a surviving outer Neg keeps its read of the
    //         inner Neg alive), and
    //   (iii) some fma-argument slot reaches it by descending `src` links
    //         through deleted Negs (otherwise no site substitutes the
    //         read; a Neg nothing absorbs is DCE's business, not ours).
    // (ii) and (iii) are mutually recursive: dropping a Neg pins its source
    // (the dropped Neg is now a surviving reader of it) and shortens every
    // chain that descended through it. Compute the GREATEST fixpoint:
    // start from every `Other`-free Neg and remove the members that
    // violate (ii) or (iii) until stable. The loop terminates because the
    // set only shrinks (at most one productive round per Neg). The edge
    // directions are load-bearing in both directions at once: poisoning a
    // Neg from its SOURCE (the direction this pass first shipped with)
    // deletes the inner link of a chain whose outer link survives — a
    // dangling use; and poisoning only from readers (the one-line repair)
    // still deletes a Neg whose surviving reader no fma chain reaches. The
    // reader closure (ii) and the site reachability (iii) must be computed
    // TOGETHER; tools/ir_shape_check.py enumerates the chain shape space
    // and keeps both failure modes pinned as negative controls.
    let mut absorbable: FxHashSet<u32> = neg_use_kinds
        .iter()
        .filter(|(_, kinds)| !kinds.iter().any(|k| *k == NegUseKind::Other))
        .map(|(&vid, _)| vid)
        .collect();
    // Reader adjacency, built once: src Neg id -> every Neg reading it.
    // (A Neg with NO read at all has no `neg_use_kinds` entry, so it never
    // enters `absorbable`: dead code is DCE's business, and excluding it
    // also pins its source through (ii) — the conservative and correct
    // outcome, since the peel must not strand a reader it never rewrote.)
    let mut readers: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (&vid, &(nb, ni)) in &float_negs {
        if let Instruction::UnaryOp {
            src: Operand::Value(s),
            ..
        } = &func.blocks[nb].instructions[ni]
        {
            if float_negs.contains_key(&s.0) {
                readers.entry(s.0).or_default().push(vid);
            }
        }
    }
    // Seeds for (iii): every Neg an fma-argument slot reads (the
    // classification already rejected width mismatches as `Other`).
    let fma_arg_reads: Vec<u32> = neg_use_kinds
        .iter()
        .filter(|(_, kinds)| kinds.iter().any(|k| *k == NegUseKind::FmaArg))
        .map(|(&vid, _)| vid)
        .collect();
    loop {
        // (iii): the Negs the sites actually rewrite through, given the
        // current delete set — from each seed, descend `src` links while
        // the Neg stays deleted. This is exactly the walk Pass 2 performs.
        let mut reached: FxHashSet<u32> = FxHashSet::default();
        let mut stack = fma_arg_reads.clone();
        while let Some(vid) = stack.pop() {
            if !absorbable.contains(&vid) || !reached.insert(vid) {
                continue;
            }
            if let Some(&(nb, ni)) = float_negs.get(&vid) {
                if let Instruction::UnaryOp {
                    src: Operand::Value(s),
                    ..
                } = &func.blocks[nb].instructions[ni]
                {
                    if float_negs.contains_key(&s.0) {
                        stack.push(s.0);
                    }
                }
            }
        }
        let mut drop: Vec<u32> = Vec::new();
        for &vid in &absorbable {
            let stranded_reader = readers
                .get(&vid)
                .is_some_and(|rs| rs.iter().any(|r| !absorbable.contains(r)));
            if !reached.contains(&vid) || stranded_reader {
                drop.push(vid);
            }
        }
        if drop.is_empty() {
            break;
        }
        for vid in drop {
            absorbable.remove(&vid);
        }
    }

    // ---- Pass 2 (decisions, still immutable): peel each fma site. ----
    let mut decisions: Vec<Peel> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic { op, args, .. } = inst else {
                continue;
            };
            // Both the plain and the already-Signed spellings peel: the
            // families compose by XOR, so a site the vectorizer's scalar
            // remainder mirror already signed can still shed a materialised
            // Neg, and a double peel that cancels normalises back to plain.
            // This is what makes the `FmaArg` classification true — every
            // read so classified really does disappear with the rewrite.
            let (expected_ty, mut negate_product, mut negate_addend) = match op {
                IntrinsicOp::FmaScalarF32 => (IrType::F32, false, false),
                IntrinsicOp::FmaScalarF64 => (IrType::F64, false, false),
                IntrinsicOp::FmaScalarF32Signed(np, na) => (IrType::F32, *np, *na),
                IntrinsicOp::FmaScalarF64Signed(np, na) => (IrType::F64, *np, *na),
                _ => continue,
            };
            if args.len() != 3 {
                continue;
            }
            let mut absorbed: Vec<u32> = Vec::new();
            let mut new_args = Vec::with_capacity(3);
            for (pos, arg) in args.iter().enumerate() {
                // Peel the (possibly chained) absorbable Negs off this
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
                    // Absorbability: every read of this Neg disappears with
                    // the rewrite (fma argument slots that all peel it, plus
                    // chain-internal reads by other absorbed Negs). A Neg
                    // with any other reader stays materialised — peeling it
                    // here would change its NaN-sign semantics without
                    // deleting the vxorpd.
                    if !absorbable.contains(&vid) {
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
                    // Recorded outermost-first (the walk descends outer →
                    // inner). Deletion does not care about the order, but
                    // any future refinement that RETARGETS a surviving
                    // reader must process this list deepest-first —
                    // retargeting an outer reader first would hand it the
                    // not-yet-rewritten inner chain and reintroduce a
                    // use-before-def.
                    absorbed.push(vid);
                }
                new_args.push(current);
            }
            if absorbed.is_empty() {
                continue;
            }
            // The site's family with every absorbed flip folded in; a full
            // cancel lands on the plain spelling (the doc rule: no
            // degenerate Signed(false, false) is ever created).
            let new_op = match expected_ty {
                IrType::F32 => match (negate_product, negate_addend) {
                    (false, false) => IntrinsicOp::FmaScalarF32,
                    (np, na) => IntrinsicOp::FmaScalarF32Signed(np, na),
                },
                _ => match (negate_product, negate_addend) {
                    (false, false) => IntrinsicOp::FmaScalarF64,
                    (np, na) => IntrinsicOp::FmaScalarF64Signed(np, na),
                },
            };
            decisions.push(Peel {
                block: bi,
                inst: ii,
                args: new_args,
                op: new_op,
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
            *op = dec.op;
        }
    }
    // The precondition the sweep relies on, made self-enforcing: the sweep
    // deletes by value id with no residual-use check, so the decision pass
    // must guarantee that every read of every absorbed Neg disappears with
    // the rewrite. Pass 1c's fixpoint certifies it for the classification;
    // this re-derives it from the ACTUAL decision set (one pass over the
    // reader adjacency) so that a future edit desynchronising Pass 1c from
    // Pass 2's walk — or introducing a new site kind that reads Negs
    // without peeling them — fails here, in the test suite, instead of
    // shipping a dangling use to codegen. Unconditional (not a
    // debug_assert!) for two reasons: the fastbuild/test profile compiles
    // with debug-assertions off, so a debug_assert would never fire in the
    // very builds that run the regression suites; and this is a
    // release-miscompile class — the house discipline for those is to
    // panic loudly (see the VecRotl/VecShufd immediate guards), which is
    // exactly how the chain defect surfaced: an ICE on `value has no
    // register, stack slot, Copy, or GlobalAddr definition`.
    for &id in &absorbed_ids {
        let kinds = neg_use_kinds
            .get(&id)
            .expect("an absorbed Neg must have a use classification");
        assert!(
            !kinds.iter().any(|k| *k == NegUseKind::Other),
            "fma_neg_peel: absorbed v{id} still has a non-peeling read"
        );
        for &r in readers.get(&id).into_iter().flatten() {
            assert!(
                absorbed_ids.contains(&r),
                "fma_neg_peel: absorbed v{id} is still read by surviving v{r}"
            );
        }
    }
    // Sweep: every absorbed Neg's destination has no remaining use — the
    // fixpoint certified it and the assert above re-derived it from the
    // actual decision set — so removal is dead-code elimination with
    // the parallel source_spans discipline (see dce::sweep_block). The keep
    // decision is computed once into a bitmap so instructions and spans walk
    // the SAME positional filter. (dce's own "refuse to sweep" guard is
    // about its EXTERNAL live-flag slice desynchronising — a caller bug it
    // fails closed on; the spans discipline both passes share is the
    // clear-on-length-mismatch below, which restores the documented
    // "empty spans == no debug info" invariant instead of leaving a stale
    // vector that would desynchronise later debug emission.)
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
    fn same_value_in_two_positions_cancels_to_plain() {
        // fma(n, n, c) with n = -a: both product positions read the SAME
        // Neg, both uses are absorbable fma argument slots, the two flips
        // cancel -> plain fma(a, a, c), the Neg dies. GCC folds the same
        // spelling; the old single-use grammar left it materialised.
        let mut m = module(
            vec![neg(1, 0, IrType::F64), fma(2, v(1), v(1), v(4), true)],
            5,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(0), v(0), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn shared_negation_across_two_fma_sites_peels_both() {
        // The phi-diamond shape (the session's headline): one -a and one -c
        // shared by TWO fma sites. All four reads are absorbable argument
        // slots, so both sites peel and BOTH Negs die — GCC's two
        // vfnmsub, not two vxorpd + two vfmadd.
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 4, IrType::F64),
                fma(2, v(1), v(3), v(5), true),
                fma(6, v(1), v(7), v(5), true),
                Instruction::BinOp {
                    dest: crate::ir::reexports::Value(8),
                    op: crate::ir::reexports::IrBinOp::Add,
                    lhs: v(2),
                    rhs: v(6),
                    ty: IrType::F64,
                },
            ],
            9,
        );
        // The helper returns v(2); extend the return to the sum so both
        // fma results stay live.
        m.functions[0].blocks[0].terminator = Terminator::Return(Some(v(8)));
        assert_eq!(peel(&mut m), 2);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 3);
        for (i, want) in [(0, (true, true)), (1, (true, true))] {
            match &insts[i] {
                Instruction::Intrinsic { op, args, .. } => {
                    assert_eq!(
                        *op,
                        IntrinsicOp::FmaScalarF64Signed(want.0, want.1),
                        "site {i}"
                    );
                    // Both sites read the PRE-negation sources.
                    assert!(args.contains(&v(0)), "site {i} must read a");
                    assert!(args.contains(&v(4)), "site {i} must read c");
                    assert!(!args.contains(&v(1)) && !args.contains(&v(5)));
                }
                other => panic!("expected intrinsic, got {other:?}"),
            }
        }
    }

    #[test]
    fn shared_negation_with_external_reader_stays_materialised() {
        // -a read by two fma sites AND a trailing add: the add survives
        // every rewrite, so the Neg keeps its exact materialised
        // semantics and NO site peels (the discipline: the NaN-sign
        // latitude is only taken when the vxorpd actually dies).
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
                fma(7, v(1), v(3), v(4), true),
                reader,
            ],
            8,
        );
        m.functions[0].blocks[0].terminator = Terminator::Return(Some(v(6)));
        assert_eq!(peel(&mut m), 0);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 4);
        match &insts[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(1), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn cross_block_negation_dominating_both_sites_peels() {
        // The Neg lives in the entry block, the fma in a dominated block:
        // the rewrite substitutes the Neg's SOURCE, whose definition
        // dominates the Neg, which dominates the use — transitivity is
        // the whole soundness argument, and it is block-agnostic.
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
            instructions: vec![neg(1, 0, IrType::F64)],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![fma(2, v(1), v(3), v(4), true)],
            terminator: Terminator::Return(Some(v(2))),
            source_spans: vec![],
        });
        func.next_value_id = 5;
        let mut module = IrModule::new();
        module.functions.push(func);
        assert_eq!(peel(&mut module), 1);
        match &module.functions[0].blocks[1].instructions[0] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, false));
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
        assert!(module.functions[0].blocks[0].instructions.is_empty());
    }

    #[test]
    fn chained_shared_negation_peels_transitively_at_both_sites() {
        // Two sites read the same DOUBLE negation: the outer Neg's two
        // reads are fma slots, the inner Neg's only read is the outer
        // chain link — all absorbable, everything collapses.
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 1, IrType::F64),
                fma(2, v(5), v(3), v(4), true),
                fma(6, v(5), v(3), v(4), true),
                Instruction::BinOp {
                    dest: crate::ir::reexports::Value(8),
                    op: crate::ir::reexports::IrBinOp::Add,
                    lhs: v(2),
                    rhs: v(6),
                    ty: IrType::F64,
                },
            ],
            9,
        );
        m.functions[0].blocks[0].terminator = Terminator::Return(Some(v(8)));
        assert_eq!(peel(&mut m), 2);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 3);
        for i in 0..2 {
            match &insts[i] {
                Instruction::Intrinsic { op, args, .. } => {
                    // (-(-a)) at a product slot: two flips -> plain family,
                    // reading a directly.
                    assert_eq!(*op, IntrinsicOp::FmaScalarF64, "site {i}");
                    assert_eq!(args, &vec![v(0), v(3), v(4)], "site {i}");
                }
                other => panic!("expected intrinsic, got {other:?}"),
            }
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
    fn chained_negation_with_surviving_outer_reader_is_never_deleted() {
        // The chain defect this pass shipped with, as a shape: the fma site
        // reads the INNER Neg; the OUTER Neg is read by an add. The outer
        // must stay materialised (the add pins it), and its surviving read
        // of the inner pins the INNER too — deleting the inner underneath
        // the outer's feet is a dangling use, which reached codegen as an
        // ICE before the fixpoint was corrected. Note the direction: the
        // site must read the INNER link for this to be the defect shape
        // (a site reading the outer is the safe mirror, pinned directly).
        //   t = -x; u = -t; r = fma(t, b, c); return r + u;
        let reader = Instruction::BinOp {
            dest: crate::ir::reexports::Value(6),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(2),
            rhs: v(5),
            ty: IrType::F64,
        };
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),         // t = -x
                neg(5, 1, IrType::F64),         // u = -t (reads t)
                fma(2, v(1), v(3), v(4), true), // r = fma(t, b, c) — INNER
                reader,                         // r + u — the Other read of u
            ],
            7,
        );
        m.functions[0].blocks[0].terminator = Terminator::Return(Some(v(6)));
        assert_eq!(peel(&mut m), 0);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 4);
        // Both Negs still present and the intrinsic stays PLAIN reading the
        // materialised t: no latitude is taken on a mask that survives.
        assert!(matches!(insts[0], Instruction::UnaryOp { dest, .. } if dest.0 == 1));
        assert!(matches!(insts[1], Instruction::UnaryOp { dest, .. } if dest.0 == 5));
        match &insts[2] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(1), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn chained_negation_with_dead_outer_reader_stays_put() {
        // The second negative control from the shape enumeration: the outer
        // Neg is read by NOTHING (DCE would normally have removed it before
        // the peel — construct the IR directly). The peel must still not
        // delete the inner link underneath it: deletion requires every
        // reader to be deleted too, and a reader no fma chain reaches is
        // never deleted. Dead code is DCE's business, not the peel's.
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                neg(5, 1, IrType::F64),         // read by nothing
                fma(2, v(1), v(3), v(4), true), // reads the INNER
            ],
            6,
        );
        assert_eq!(peel(&mut m), 0);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 3);
        assert!(matches!(insts[0], Instruction::UnaryOp { dest, .. } if dest.0 == 1));
        assert!(matches!(insts[1], Instruction::UnaryOp { dest, .. } if dest.0 == 5));
    }

    #[test]
    fn outer_neg_peels_when_the_inner_neg_is_pinned() {
        // The fold the source-poisoned rule missed: the INNER Neg is pinned
        // by an external add, the site reads the OUTER Neg. The outer's
        // only read is the site, so its sign-mask dies and the latitude is
        // available; the site peels exactly one level and reads the
        // materialised inner:
        //   t = -x; r = fma(-t, b, c); return r + t;
        // compiles to vxorpd + vfnmadd + vaddsd — one sign mask, not two —
        // bit-exact against GCC (verified on the metal in the session
        // record; the landed rule kept both masks).
        let reader = Instruction::BinOp {
            dest: crate::ir::reexports::Value(6),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(2),
            rhs: v(1),
            ty: IrType::F64,
        };
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),         // t = -x, pinned by the add
                neg(5, 1, IrType::F64),         // -t, read only by the site
                fma(2, v(5), v(3), v(4), true), // r = fma(-t, b, c) — OUTER
                reader,
            ],
            7,
        );
        m.functions[0].blocks[0].terminator = Terminator::Return(Some(v(6)));
        assert_eq!(peel(&mut m), 1);
        let insts = &m.functions[0].blocks[0].instructions;
        assert_eq!(insts.len(), 3);
        // The inner Neg stays materialised; the site now reads it with the
        // product negated (the dying OUTER mask carried the sign).
        assert!(matches!(insts[0], Instruction::UnaryOp { dest, .. } if dest.0 == 1));
        match &insts[1] {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, false));
                assert_eq!(args, &vec![v(1), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn already_signed_site_peels_and_normalises_to_plain() {
        // The XOR composition: a site the vectorizer's scalar remainder
        // mirror already signed peels too. Signed(true, false) absorbing
        // the Neg on its multiplier slot flips the product flag back — the
        // full cancel NORMALISES to the plain spelling (no degenerate
        // Signed(false, false) is ever created). This is what makes the
        // FmaArg classification true rather than coincidentally
        // conservative — the read really does disappear.
        let mut m = module(
            vec![
                neg(1, 0, IrType::F64),
                Instruction::Intrinsic {
                    dest: Some(crate::ir::reexports::Value(2)),
                    op: IntrinsicOp::FmaScalarF64Signed(true, false),
                    dest_ptr: None,
                    args: vec![v(1), v(3), v(4)],
                },
            ],
            5,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64);
                assert_eq!(args, &vec![v(0), v(3), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn already_signed_site_peels_by_xor_into_addend() {
        // The same composition on the addend slot, where the flags do NOT
        // cancel: Signed(true, false) absorbing the addend Neg XORs to
        // Signed(true, true) — the vfnmsub family — reading the
        // pre-negation addend.
        let mut m = module(
            vec![
                neg(1, 4, IrType::F64),
                Instruction::Intrinsic {
                    dest: Some(crate::ir::reexports::Value(2)),
                    op: IntrinsicOp::FmaScalarF64Signed(true, false),
                    dest_ptr: None,
                    args: vec![v(3), v(5), v(1)],
                },
            ],
            6,
        );
        assert_eq!(peel(&mut m), 1);
        match sole_inst(&m) {
            Instruction::Intrinsic { op, args, .. } => {
                assert_eq!(*op, IntrinsicOp::FmaScalarF64Signed(true, true));
                assert_eq!(args, &vec![v(3), v(5), v(4)]);
            }
            other => panic!("expected intrinsic, got {other:?}"),
        }
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
